use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Stylize,
    symbols::border,
    text::Line,
    widgets::{Block, Paragraph, Row, Table, Widget},
    DefaultTerminal, Frame,
};
use rts_alloc::Allocator;
use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Args {
    /// The path to the allocator file.
    #[clap(short, long)]
    path: PathBuf,
    #[command(subcommand)]
    subcommand: Option<SubCommand>,
}

const DEFAULT_SLAB_SIZE: u32 = 2 * 1024 * 1024; // 2 MiB

#[derive(Clone, Debug, Subcommand)]
enum SubCommand {
    Create {
        /// The number of workers.
        #[clap(long)]
        num_workers: u32,
        /// The total size of the allocator.
        #[clap(long)]
        file_size: usize,
        /// The size of each slab.
        #[clap(long, default_value_t = DEFAULT_SLAB_SIZE)]
        slab_size: u32,
        /// Delete existing file if present.
        #[clap(long, default_value_t = false)]
        delete_existing: bool,
    },
}

fn main() {
    let Args { path, subcommand } = Args::parse();

    let allocator = match subcommand {
        Some(SubCommand::Create {
            num_workers,
            file_size,
            slab_size,
            delete_existing,
        }) => {
            if delete_existing && path.exists() {
                std::fs::remove_file(&path).expect("Failed to delete existing allocator file");
            }
            rts_alloc::create_allocator(&path, num_workers, slab_size, file_size)
                .expect("Failed to create allocator")
        }
        None => rts_alloc::join_allocator(&path).expect("Failed to join allocator"),
    };

    // Setup TUI
    let mut terminal = ratatui::init();
    let _ = App {
        allocator,
        slab_offset: AtomicUsize::new(0),
        exit: false,
    }
    .run(&mut terminal);
    ratatui::restore();
}

pub struct App {
    allocator: Allocator,
    slab_offset: AtomicUsize,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        const TICK_RATE: Duration = Duration::from_millis(100);
        let mut last_tick = Instant::now();

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;

            let timeout = TICK_RATE
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            self.handle_events(timeout)?;

            if last_tick.elapsed() >= TICK_RATE {
                last_tick = Instant::now();
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self, timeout: Duration) -> io::Result<()> {
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    self.handle_key_event(key_event)
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Left => self.slab_offset(false),
            KeyCode::Right => self.slab_offset(true),
            _ => {}
        }
    }

    fn slab_offset(&mut self, increment: bool) {
        if increment {
            self.slab_offset.fetch_add(1, Ordering::Relaxed)
        } else {
            self.slab_offset.fetch_sub(1, Ordering::Relaxed)
        };

        // Make sure the slab offset is within bounds.
        self.slab_offset.store(
            self.slab_offset
                .load(Ordering::Relaxed)
                .clamp(0, self.allocator.header().num_slabs as usize - 1),
            Ordering::Relaxed,
        );
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from("Allocator").bold();
        let instructions = Line::from(vec!["Press 'q' to quit".into()]);
        let outer_block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);
        let inner = outer_block.inner(area);
        outer_block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(9),
                Constraint::Length(5),
                Constraint::Min(5),
            ])
            .split(inner);

        let header = self.allocator.header();

        // -- Header Section
        let header_rows = vec![
            Row::new(vec!["magic".into(), format!("0x{:016X}", header.magic)]),
            Row::new(vec!["version".into(), header.version.to_string()]),
            Row::new(vec!["num_workers".into(), header.num_workers.to_string()]),
            Row::new(vec!["num_slabs".into(), header.num_slabs.to_string()]),
            Row::new(vec!["slab_size".into(), header.slab_size.to_string()]),
            Row::new(vec!["slab_offset".into(), header.slab_offset.to_string()]),
            Row::new(vec![
                "global_free_stack".into(),
                header.global_free_stack.load(Ordering::Relaxed).to_string(),
            ]),
        ];
        let header_block = Block::bordered().title("Header").border_set(border::PLAIN);
        let header_table = Table::new(
            header_rows,
            &[Constraint::Length(20), Constraint::Length(30)],
        )
        .block(header_block)
        .column_spacing(2);
        header_table.render(chunks[0], buf);

        // -- Padding Section
        let padding_block = Block::bordered().title("Padding");
        let padding = Paragraph::new("... padding ...").block(padding_block);
        padding.render(chunks[1], buf);

        // -- Slab section
        let slabs_block = Block::bordered().title("Slabs");
        let slabs_inner = slabs_block.inner(chunks[2]);
        slabs_block.render(chunks[2], buf);
        {
            let area = &slabs_inner;

            let num_slabs = header.num_slabs as usize;
            let slab_width = 14;
            let slab_height = 3;
            let slabs_per_row = usize::from(area.width / slab_width);

            // We will have a partial view if the number of slabs
            // is greater than the number of slabs that can fit in a row.
            let (starting_index, ending_index) = if num_slabs > slabs_per_row {
                // Adjust slab offset if necessary to avoid needing to scroll back many times.
                let slab_offset = self.slab_offset.load(Ordering::Relaxed);
                if slab_offset + slabs_per_row > num_slabs {
                    self.slab_offset
                        .store(num_slabs.saturating_sub(slabs_per_row), Ordering::Relaxed);
                }
                let slab_offset = self.slab_offset.load(Ordering::Relaxed);
                (slab_offset, slab_offset + slabs_per_row)
            } else {
                (0, num_slabs)
            };

            for (index, slab_index) in (starting_index..ending_index).enumerate() {
                let x = area.x + (index as u16 * slab_width);
                let rect = Rect::new(x, area.y, slab_width, slab_height);

                if (index == 0 && slab_index != 0)
                    || (index == (slabs_per_row - 1) && slab_index != (num_slabs - 1))
                {
                    let slab_block = Block::bordered().border_set(border::PLAIN);
                    let slab_value = Paragraph::new("...")
                        .block(slab_block)
                        .alignment(Alignment::Center);
                    slab_value.render(rect, buf);
                } else {
                    // Each slab in its own mini-block
                    let slab_block = Block::bordered()
                        .title(format!("Slab {}", slab_index))
                        .border_set(border::PLAIN);
                    let slab_value =
                        unsafe { self.allocator.slab(slab_index as u32).cast::<u32>().read() };
                    let slab_value = Paragraph::new(slab_value.to_string())
                        .block(slab_block)
                        .alignment(Alignment::Center);
                    slab_value.render(rect, buf);
                }
            }

            // let (start_index, end_index, partial) = if num_slabs > slabs_per_row {
            //     let slab_offset = if self.slab_offset + slabs_per_row > num_slabs {
            //         num_slabs.saturating_sub(slabs_per_row)
            //     } else {
            //         self.slab_offset
            //     };
            //     (
            //         slab_offset,
            //         (slab_offset + slabs_per_row - 1).min(num_slabs), // -1 for trailing ... block
            //         true,
            //     )
            // } else {
            //     (0, num_slabs, false)
            // };

            // for (index, slab_index) in (start_index..end_index).enumerate() {
            //     let x = area.x + (index as u16 * slab_width);
            //     let rect = Rect::new(x, area.y, slab_width, slab_height);

            //     // Each slab in its own mini-block
            //     let slab_block = Block::bordered()
            //         .title(format!("Slab {}", slab_index))
            //         .border_set(border::PLAIN);
            //     let slab_value =
            //         unsafe { self.allocator.slab(slab_index as u32).cast::<u32>().read() };
            //     let slab_value = Paragraph::new(slab_value.to_string())
            //         .block(slab_block)
            //         .alignment(Alignment::Center);
            //     slab_value.render(rect, buf);
            // }

            // if partial {
            //     let x = area.x + ((slabs_per_row - 1) as u16 * slab_width);
            //     let rect = Rect::new(x, area.y, slab_width, slab_height);
            //     // Trailing block for partial slabs
            //     let partial_block = Block::bordered().title("...").border_set(border::PLAIN);
            //     let partial_value = Paragraph::new("...")
            //         .block(partial_block)
            //         .alignment(Alignment::Center);
            //     partial_value.render(rect, buf);
            // }
        }
    }
}

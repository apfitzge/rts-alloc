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
        worker_offset: AtomicUsize::new(0),
        slab_offset: AtomicUsize::new(0),
        selected_section: Section::Slabs, // default to Slabs because there will more often be more of those
        exit: false,
    }
    .run(&mut terminal);
    ratatui::restore();
}

enum Section {
    Workers,
    Slabs,
}

pub struct App {
    allocator: Allocator,
    worker_offset: AtomicUsize,
    slab_offset: AtomicUsize,
    selected_section: Section,
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
            KeyCode::Left => self.scroll(false),
            KeyCode::Right => self.scroll(true),
            KeyCode::Down => self.select(false),
            KeyCode::Up => self.select(true),
            _ => {}
        }
    }

    fn scroll(&mut self, increment: bool) {
        let header = self.allocator.header();
        let (offset, max) = match self.selected_section {
            Section::Workers => (&self.worker_offset, header.num_workers),
            Section::Slabs => (&self.slab_offset, header.num_slabs),
        };

        if increment {
            offset.fetch_add(1, Ordering::Relaxed);
        } else {
            let prev = offset.fetch_sub(1, Ordering::Relaxed);
            if prev == 0 {
                offset.store(0, Ordering::Relaxed);
            }
        };

        // Make sure the offset is within bounds.
        offset.store(
            offset.load(Ordering::Relaxed).clamp(0, max as usize - 1),
            Ordering::Relaxed,
        );
    }

    fn select(&mut self, _up: bool) {
        match self.selected_section {
            Section::Workers => self.selected_section = Section::Slabs,
            Section::Slabs => self.selected_section = Section::Workers,
        }
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

        let num_workers = self.allocator.header().num_workers;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(9),
                Constraint::Length(num_workers as u16 + 3),
                Constraint::Length(5),
                Constraint::Min(5),
            ])
            .split(inner);

        self.render_header(chunks[0], buf);
        self.render_workers(chunks[1], buf);
        self.render_padding(chunks[2], buf);
        self.render_slabs(chunks[3], buf);
    }
}

impl App {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let header = self.allocator.header();
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
        header_table.render(area, buf);
    }

    fn render_workers(&self, area: Rect, buf: &mut Buffer) {
        let header = self.allocator.header();
        let workers_block =
            Block::bordered().title(if matches!(self.selected_section, Section::Workers) {
                "Workers*"
            } else {
                "Workers"
            });
        let workers_inner = workers_block.inner(area);
        workers_block.render(area, buf);

        let area = &workers_inner;
        let num_workers = header.num_workers as usize;
        let worker_width = 30;
        let worker_height = 5;
        let workers_per_row = usize::from(area.width / worker_width);

        // We will have a partial view if the number of workers
        // is greater than the number of workers that can fit in a row.
        let (starting_index, ending_index) = if num_workers > workers_per_row {
            // Adjust worker offset if necessary to avoid needing to scroll back many times.
            let worker_offset = self.worker_offset.load(Ordering::Relaxed);
            if worker_offset + workers_per_row > num_workers {
                self.worker_offset.store(
                    num_workers.saturating_sub(workers_per_row) as usize,
                    Ordering::Relaxed,
                );
            }
            let worker_offset = self.worker_offset.load(Ordering::Relaxed);
            (worker_offset, worker_offset + workers_per_row as usize)
        } else {
            (0, num_workers as usize)
        };

        for (index, worker_index) in (starting_index..ending_index).enumerate() {
            let x = area.x + (index as u16 * worker_width);
            let rect = Rect::new(x, area.y, worker_width, worker_height);

            if (index == 0 && worker_index != 0)
                || (index == (workers_per_row - 1) && worker_index != (num_workers - 1))
            {
                self.render_worker_continuation(rect, buf);
            } else {
                self.render_worker(worker_index, rect, buf);
            }
        }
    }

    fn render_worker(&self, worker_index: usize, area: Rect, buf: &mut Buffer) {
        let worker_block = Block::bordered()
            .title(format!("Worker {}", worker_index))
            .border_set(border::PLAIN);
        let worker_state = unsafe { self.allocator.worker_state(worker_index as usize).as_ref() };
        let partial_slabs_head = worker_state.partial_slabs_head.load(Ordering::Relaxed);
        let full_slabs_head = worker_state.full_slabs_head.load(Ordering::Relaxed);
        let table = Table::new(
            vec![
                Row::new(vec![
                    "Partial Slabs".into(),
                    format!("{}", partial_slabs_head),
                ]),
                Row::new(vec!["Full Slabs".into(), format!("{}", full_slabs_head)]),
            ],
            &[
                Constraint::Length(16), // label column width
                Constraint::Min(10),    // value column, right-aligned
            ],
        )
        .block(worker_block)
        .column_spacing(1);

        table.render(area, buf);
    }

    fn render_worker_continuation(&self, area: Rect, buf: &mut Buffer) {
        let worker_block = Block::bordered().border_set(border::PLAIN);
        let worker_value = Paragraph::new("...")
            .block(worker_block)
            .alignment(Alignment::Center);
        worker_value.render(area, buf);
    }

    fn render_padding(&self, area: Rect, buf: &mut Buffer) {
        let padding_block = Block::bordered().title("Padding");
        let padding = Paragraph::new("... padding ...").block(padding_block);
        padding.render(area, buf);
    }

    fn render_slabs(&self, area: Rect, buf: &mut Buffer) {
        let header = self.allocator.header();
        let slabs_block =
            Block::bordered().title(if matches!(self.selected_section, Section::Slabs) {
                "Slabs*"
            } else {
                "Slabs"
            });
        let slabs_inner = slabs_block.inner(area);
        slabs_block.render(area, buf);
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
                    self.render_slab_continuation(rect, buf);
                } else {
                    self.render_slab(slab_index, rect, buf);
                }
            }
        }
    }

    fn render_slab(&self, slab_index: usize, area: Rect, buf: &mut Buffer) {
        let slab_block = Block::bordered()
            .title(format!("Slab {}", slab_index))
            .border_set(border::PLAIN);
        let slab_value = unsafe { self.allocator.slab(slab_index as u32).cast::<u32>().read() };
        let slab_value = Paragraph::new(slab_value.to_string())
            .block(slab_block)
            .alignment(Alignment::Center);
        slab_value.render(area, buf);
    }

    fn render_slab_continuation(&self, area: Rect, buf: &mut Buffer) {
        let slab_block = Block::bordered().border_set(border::PLAIN);
        let slab_value = Paragraph::new("...")
            .block(slab_block)
            .alignment(Alignment::Center);
        slab_value.render(area, buf);
    }
}

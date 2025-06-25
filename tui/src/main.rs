use std::{io, path::PathBuf, sync::atomic::Ordering};

use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Stylize,
    symbols::border,
    text::Line,
    widgets::{Block, Paragraph, Row, Table, Widget},
    DefaultTerminal, Frame,
};
use rts_alloc::Allocator;

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
        exit: false,
    }
    .run(&mut terminal);
    ratatui::restore();
}

pub struct App {
    allocator: Allocator,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            _ => {}
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

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(5),
                Constraint::Min(0),
            ])
            .split(inner);

        let header = self.allocator.header();

        // -- Header Section
        let header_rows = vec![
            Row::new(vec!["magic".into(), format!("0x{:016X}", header.magic)]),
            Row::new(vec!["version".into(), header.version.to_string()]),
            Row::new(vec!["num_workers".into(), header.num_workers.to_string()]),
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
    }
}

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Args {
    /// The path to the allocator file.
    #[clap(short, long)]
    path: PathBuf,
    #[command(subcommand)]
    subcommand: SubCommand,
}

#[derive(Clone, Debug, Subcommand)]
enum SubCommand {
    PopFreeSlab { worker_id: u32 },
    ReturnTheSlab { slab_index: u32 },
}

fn main() {
    let args = Args::parse();

    // Join the allocator.
    let allocator = match rts_alloc::join_allocator(args.path) {
        Ok(allocator) => allocator,
        Err(e) => {
            eprintln!("Failed to join allocator: {:?}", e);
            return;
        }
    };

    match args.subcommand {
        SubCommand::PopFreeSlab { worker_id } => match allocator.try_pop_free_slab() {
            Some(slab_index) => println!("Worker {} popped slab index: {}", worker_id, slab_index),
            None => eprintln!("Failed to pop free slab"),
        },
        SubCommand::ReturnTheSlab { slab_index } => {
            unsafe { allocator.return_the_slab(slab_index) };
            println!("Returned slab index: {}", slab_index);
        }
    }
}

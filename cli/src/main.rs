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
    TakeFreeSlab {
        worker_id: usize,
        size_class_index: usize,
    },
    ReturnTheSlab {
        slab_index: u32,
    },
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
        SubCommand::TakeFreeSlab {
            worker_id,
            size_class_index,
        } => {
            if allocator.take_free_slab(worker_id, size_class_index) {
                println!(
                    "Worker {} popped slab with size class {}",
                    worker_id, size_class_index
                );
            } else {
                eprintln!("Failed to take free slab");
            }
        }
        SubCommand::ReturnTheSlab { slab_index } => {
            unsafe { allocator.return_the_slab(slab_index) };
            println!("Returned slab index: {}", slab_index);
        }
    }
}

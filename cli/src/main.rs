use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rts_alloc::WorkerAssignedAllocator;

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
    Allocate { worker_id: u32, size: u32 },
    Free { worker_id: u32, offset: u32 },
    Simulate { worker_id: u32 },
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
        SubCommand::Allocate { worker_id, size } => {
            let allocator = WorkerAssignedAllocator::new(allocator, worker_id);
            match allocator.allocate(size) {
                Some(ptr) => {
                    let offset =
                        unsafe { ptr.byte_offset_from_unsigned(allocator.allocator.header) };
                    println!(
                        "Worker {} allocated {} bytes at offset {} (0x{:x})",
                        worker_id,
                        size,
                        offset,
                        ptr.as_ptr() as usize
                    );
                }
                None => {
                    eprintln!("Failed to allocate {} bytes for worker {}", size, worker_id);
                }
            }
        }
        SubCommand::Free { worker_id, offset } => {
            let allocator = WorkerAssignedAllocator::new(allocator, worker_id);
            let ptr = unsafe {
                allocator
                    .allocator
                    .header
                    .byte_add(offset as usize)
                    .cast::<u8>()
            };
            unsafe {
                allocator.free(ptr);
            }
            println!(
                "Worker {} freed memory at offset {} (0x{:x})",
                worker_id, offset, offset
            );
        }
        SubCommand::Simulate { worker_id } => {
            let allocator = WorkerAssignedAllocator::new(allocator, worker_id);

            // Choose an arbitrary rotation of sizes to allocate in a loop, then free them.
            let sizes = [
                3521, 2186, 171, 2967, 347, 2011, 1552, 3900, 3124, 3857, 1190, 1427, 2856, 2484,
                2255, 3989, 777, 144,
            ];
            let mut allocations = Vec::with_capacity(sizes.len());
            loop {
                for size in sizes {
                    if let Some(ptr) = allocator.allocate(size) {
                        allocations.push(ptr);
                    }
                }
                // Free all allocations
                for ptr in allocations.drain(..) {
                    unsafe {
                        allocator.free(ptr);
                    }
                }
            }
        }
    }
}

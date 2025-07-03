use std::{path::PathBuf, time::Instant};

use clap::{Parser, Subcommand};
use rts_alloc::{size_classes::SIZE_CLASSES, WorkerAssignedAllocator};

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
    Allocate {
        worker_id: u32,
        size: u32,
    },
    Free {
        worker_id: u32,
        offset: u32,
    },
    ClearWorker {
        worker_id: u32,
    },
    Simulate {
        worker_id: u32,
        #[clap(long, default_value_t = false)]
        hold_slabs: bool,
    },
    SimulateRemoteFree {
        allocate: u32,
        free: u32,
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
        SubCommand::ClearWorker { worker_id } => {
            allocator.clear_worker(worker_id);
            println!("Worker {} cleared its allocations", worker_id);
        }
        SubCommand::Simulate {
            worker_id,
            hold_slabs,
        } => {
            let allocator = WorkerAssignedAllocator::new(allocator, worker_id);
            allocator.allocator.clear_worker(worker_id);
            if hold_slabs {
                // Allocate one per size-class to hold slabs without returning them
                // in the loop below.
                for size in SIZE_CLASSES {
                    let _ = allocator.allocate(size);
                }
            }

            // Choose an arbitrary rotation of sizes to allocate in a loop, then free them.
            let sizes = [
                3521, 2186, 171, 2967, 347, 2011, 1552, 3900, 3124, 3857, 1190, 1427, 2856, 2484,
                2255, 3989, 777, 144,
            ];
            let mut allocations = Vec::with_capacity(sizes.len());
            let mut num_allocated = 0;
            let mut bytes_allocated = 0;
            let mut now = Instant::now();
            loop {
                for size in sizes {
                    if let Some(ptr) = allocator.allocate(size) {
                        num_allocated += 1;
                        bytes_allocated += size;
                        allocations.push(ptr);
                    }
                }
                // Free all allocations
                for ptr in allocations.drain(..) {
                    unsafe {
                        allocator.free(ptr);
                    }
                }

                let new_now = Instant::now();
                if new_now.duration_since(now).as_secs() >= 1 {
                    println!(
                        "{num_allocated} a/s ({} MiB/s)",
                        bytes_allocated / (1024 * 1024)
                    );
                    now = new_now;
                    num_allocated = 0;
                    bytes_allocated = 0;
                }
            }
        }
        SubCommand::SimulateRemoteFree { allocate, free } => {
            // HACK: change the worker_id of this assigned allocator to simulate remote frees.
            let mut allocator = WorkerAssignedAllocator::new(allocator, allocate);
            // Choose an arbitrary rotation of sizes to allocate in a loop, then free them.
            let sizes = [
                3521, 2186, 171, 2967, 347, 2011, 1552, 3900, 3124, 3857, 1190, 1427, 2856, 2484,
                2255, 3989, 777, 144,
            ];
            let mut allocations = Vec::with_capacity(sizes.len());
            let mut num_allocated = 0;
            let mut bytes_allocated = 0;
            let mut now = Instant::now();
            loop {
                allocator.worker_index = allocate;
                allocator.drain_remote_frees();

                let mut full = false;
                for size in sizes {
                    if let Some(ptr) = allocator.allocate(size) {
                        num_allocated += 1;
                        bytes_allocated += size;
                        allocations.push(ptr);
                    } else {
                        full = true;
                    }
                }

                // Only free if some allocations failed - i.e. ran out of space.
                if full {
                    // Free all allocations
                    allocator.worker_index = free; // Simulate remote frees by changing the worker_id
                    for ptr in allocations.drain(..) {
                        unsafe {
                            allocator.free(ptr);
                        }
                    }
                }

                let new_now = Instant::now();
                if new_now.duration_since(now).as_secs() >= 1 {
                    println!(
                        "{num_allocated} a/s ({} MiB/s)",
                        bytes_allocated / (1024 * 1024)
                    );
                    now = new_now;
                    num_allocated = 0;
                    bytes_allocated = 0;
                }
            }
        }
    }
}

use clap::{Parser, Subcommand};
use rts_alloc::{size_classes::SIZE_CLASSES, WorkerAssignedAllocator};
use std::{path::PathBuf, time::Instant};

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
        #[clap(long, default_value_t = false)]
        hold_slabs: bool,
    },
}

fn main() {
    let args = Args::parse();

    match args.subcommand {
        SubCommand::Create {
            num_workers,
            file_size,
            slab_size,
            delete_existing,
        } => {
            if delete_existing && args.path.exists() {
                if let Err(e) = std::fs::remove_file(&args.path) {
                    eprintln!("Failed to delete existing allocator file: {e:?}");
                    return;
                }
            }
            if let Err(err) =
                rts_alloc::init::create_allocator(args.path, num_workers, slab_size, file_size)
            {
                eprintln!("Failed to create allocator: {err:?}");
            };
        }
        SubCommand::Allocate { worker_id, size } => {
            let allocator = match rts_alloc::init::join_allocator(args.path) {
                Ok(allocator) => allocator,
                Err(e) => {
                    eprintln!("Failed to join allocator: {e:?}");
                    return;
                }
            };
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
                    eprintln!("Failed to allocate {size} bytes for worker {worker_id}");
                }
            }
        }
        SubCommand::Free { worker_id, offset } => {
            // Join the allocator.
            let allocator = match rts_alloc::init::join_allocator(args.path) {
                Ok(allocator) => allocator,
                Err(e) => {
                    eprintln!("Failed to join allocator: {e:?}");
                    return;
                }
            };
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
            println!("Worker {worker_id} freed memory at offset {offset} (0x{offset:x})");
        }
        SubCommand::ClearWorker { worker_id } => {
            // Join the allocator.
            let allocator = match rts_alloc::init::join_allocator(args.path) {
                Ok(allocator) => allocator,
                Err(e) => {
                    eprintln!("Failed to join allocator: {e:?}");
                    return;
                }
            };
            allocator.clear_worker(worker_id);
            println!("Worker {worker_id} cleared its allocations");
        }
        SubCommand::Simulate {
            worker_id,
            hold_slabs,
        } => {
            // Join the allocator.
            let allocator = match rts_alloc::init::join_allocator(args.path) {
                Ok(allocator) => allocator,
                Err(e) => {
                    eprintln!("Failed to join allocator: {e:?}");
                    return;
                }
            };
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
            let mut num_allocated = 0u64;
            let mut bytes_allocated = 0u64;
            let mut now = Instant::now();
            loop {
                for size in sizes {
                    if let Some(ptr) = allocator.allocate(size) {
                        num_allocated += 1;
                        bytes_allocated += u64::from(size);
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
        SubCommand::SimulateRemoteFree {
            allocate,
            free,
            hold_slabs,
        } => {
            // Join the allocator.
            let allocator = match rts_alloc::init::join_allocator(args.path) {
                Ok(allocator) => allocator,
                Err(e) => {
                    eprintln!("Failed to join allocator: {e:?}");
                    return;
                }
            };

            // HACK: change the worker_id of this assigned allocator to simulate remote frees.
            let mut allocator = WorkerAssignedAllocator::new(allocator, allocate);

            allocator.allocator.clear_worker(allocate);
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
            let mut num_allocated = 0u64;
            let mut bytes_allocated = 0u64;
            let mut now = Instant::now();
            loop {
                allocator.worker_index = allocate;
                allocator.drain_remote_frees();

                let mut full = false;
                for size in sizes {
                    if let Some(ptr) = allocator.allocate(size) {
                        num_allocated += 1;
                        bytes_allocated += u64::from(size);
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

use crate::{
    align::round_to_next_alignment_of,
    cache_aligned::CacheAligned,
    error::Error,
    global_free_stack,
    header::{Header, MAGIC, VERSION},
    index::NULL_U32,
    raw_allocator::RawAllocator,
    size_classes::{MIN_SIZE, NUM_SIZE_CLASSES},
    slab_meta::SlabMeta,
    worker_state::WorkerState,
};
use std::{
    os::fd::AsRawFd,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

pub fn create_allocator(
    file_path: impl AsRef<Path>,
    num_workers: u32,
    slab_size: u32,
    size: usize,
) -> Result<RawAllocator, Error> {
    if !slab_size.is_power_of_two() {
        return Err(Error::InvalidSlabSize);
    }

    let file_path = file_path.as_ref();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(file_path)
        .map_err(Error::IoError)?;
    file.set_len(size as u64).map_err(Error::IoError)?;

    let mmap = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };

    if mmap == libc::MAP_FAILED {
        return Err(Error::MMapError(mmap as usize));
    }

    let header_ptr = mmap as *mut Header;
    let mut header = NonNull::new(header_ptr).expect("mmap cannot be null after map_failed check");

    const WORKER_STATES_OFFSET: usize = core::mem::offset_of!(Header, worker_states);
    let worker_states_size = (num_workers as usize) * core::mem::size_of::<WorkerState>();
    const SLAB_META_ALIGNMENT: usize = core::mem::align_of::<SlabMeta>();
    let slab_meta_offset = (WORKER_STATES_OFFSET + worker_states_size + SLAB_META_ALIGNMENT - 1)
        & !(SLAB_META_ALIGNMENT - 1);

    // Each SlabMeta is aligned to SLAB_META_ALIGNMENT
    let slab_meta_size = core::mem::size_of::<SlabMeta>()
        + (slab_size / MIN_SIZE) as usize * core::mem::size_of::<u16>();
    // round to next multiple of alignment.
    let slab_meta_size: usize = round_to_next_alignment_of::<SLAB_META_ALIGNMENT>(slab_meta_size);

    // Total header and meta size - round to next multiple of slab size.
    let mask = (slab_size - 1) as usize;
    let total_meta_size = (slab_meta_offset + slab_meta_size + mask) & !mask;

    let slab_offset = total_meta_size as u32;
    let num_slabs = (size as u32 - slab_offset) / slab_size;

    unsafe {
        header.as_mut().magic = MAGIC;
        header.as_mut().version = VERSION;
        header.as_mut().num_workers = num_workers;
        header.as_mut().num_slabs = num_slabs;
        header.as_mut().slab_meta_size = slab_meta_size as u32;
        header.as_mut().slab_meta_offset = slab_meta_offset as u32;
        header.as_mut().slab_size = slab_size;
        header.as_mut().slab_offset = slab_offset;
        header.as_mut().global_free_stack = CacheAligned(AtomicU32::new(NULL_U32));
    }

    let allocator = RawAllocator { header };
    // Initialize slabs in the global free stack.
    for index in (0..num_slabs).rev() {
        unsafe { global_free_stack::return_the_slab(&allocator, index) };
    }

    // Initialize worker states.
    for worker_index in 0..num_workers {
        let mut worker_state = unsafe { allocator.worker_state(worker_index) };
        for size_index in 0..NUM_SIZE_CLASSES {
            unsafe {
                worker_state.as_mut().partial_slabs_heads[size_index]
                    .store(NULL_U32, Ordering::Relaxed);
                worker_state.as_mut().full_slabs_heads[size_index]
                    .store(NULL_U32, Ordering::Relaxed);
            }
        }
    }

    Ok(RawAllocator { header })
}

pub fn join_allocator(file_path: impl AsRef<Path>) -> Result<RawAllocator, Error> {
    let file_path = file_path.as_ref();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(file_path)
        .map_err(Error::IoError)?;

    let size = file.metadata().map_err(Error::IoError)?.len() as usize;

    let mmap = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };

    if mmap == libc::MAP_FAILED {
        return Err(Error::MMapError(mmap as usize));
    }

    let header_ptr = mmap as *mut Header;
    let header = NonNull::new(header_ptr).expect("mmap cannot be null after map_failed check");
    Ok(RawAllocator { header })
}

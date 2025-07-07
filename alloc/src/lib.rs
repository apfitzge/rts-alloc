use std::{
    os::fd::AsRawFd,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{
    align::round_to_next_alignment_of,
    cache_aligned::CacheAligned,
    error::Error,
    header::{Header, MAGIC, VERSION},
    index::NULL_U32,
    raw_allocator::RawAllocator,
    size_classes::{size_class_index, MAX_SIZE, MIN_SIZE, NUM_SIZE_CLASSES, SIZE_CLASSES},
    slab_meta::SlabMeta,
    worker_state::WorkerState,
};

mod align;
pub mod cache_aligned;
pub mod error;
pub mod free_stack;
mod global_free_stack;
pub mod header;
pub mod index;
pub mod raw_allocator;
pub mod remote_free_stack;
pub mod size_classes;
pub mod slab_meta;
mod worker_local_list;
pub mod worker_state;

pub struct WorkerAssignedAllocator {
    pub allocator: RawAllocator,
    pub worker_index: u32,
}

impl WorkerAssignedAllocator {
    pub fn new(allocator: RawAllocator, worker_index: u32) -> Self {
        assert!(worker_index < allocator.header().num_workers);
        WorkerAssignedAllocator {
            allocator,
            worker_index,
        }
    }

    pub fn allocate(&self, size: u32) -> Option<NonNull<u8>> {
        if size > MAX_SIZE {
            return None;
        }
        let size_class_index = size_class_index(size);

        // Check if there is a partial slab available for this size class.
        let worker = unsafe { self.allocator.worker_state(self.worker_index).as_ref() };
        let partial_head = &worker.partial_slabs_heads[size_class_index as usize];
        let mut slab_index = partial_head.load(Ordering::Acquire);
        if slab_index == NULL_U32 {
            // No partial slab available, try to take a free slab.
            if !self
                .allocator
                .take_free_slab(self.worker_index, size_class_index)
            {
                return None; // No free slab available.
            }
            slab_index = partial_head.load(Ordering::Acquire);
        }

        // At this point, we have a partial slab available.
        debug_assert_ne!(slab_index, NULL_U32, "partial slab head should not be NULL");

        // Pop an item from the free stack of the slab.
        let slab_meta = unsafe { self.allocator.slab_meta(slab_index).as_mut() };
        let free_stack = &mut slab_meta.free_stack;
        // SAFETY: The slab-meta is initialized with enough space for the free stack.
        let allocation_index_in_slab =
            unsafe { free_stack.pop() }.expect("partial slab should have free items");

        // If the free stack is now empty, move the slab to the full list.
        if free_stack.is_empty() {
            // Remove the slab from the partial list.
            unsafe {
                worker_local_list::remove_slab_from_list(
                    &self.allocator,
                    &worker.partial_slabs_heads[size_class_index as usize],
                    slab_index,
                );
            }

            // Push the slab into the full list.
            unsafe {
                worker_local_list::push_slab_into_list(
                    &self.allocator,
                    &worker.full_slabs_heads[size_class_index as usize],
                    slab_index,
                );
            }
        }

        let slab = unsafe { self.allocator.slab(slab_index) };
        let offset = usize::from(allocation_index_in_slab)
            * SIZE_CLASSES[size_class_index as usize] as usize;
        let ptr = unsafe { slab.byte_add(offset) };

        Some(ptr)
    }

    /// Free an allocated pointer.
    ///
    /// # Safety
    /// - `ptr` must be a valid pointer that was allocated by this allocator.
    pub unsafe fn free(&self, ptr: NonNull<u8>) {
        let offset = self.allocator.ptr_to_offset(ptr);

        let offset_from_slab_section_start = offset - self.allocator.header().slab_offset as usize;
        let slab_index =
            (offset_from_slab_section_start / self.allocator.header().slab_size as usize) as u32;
        let offset_from_slab_start = offset_from_slab_section_start
            - (slab_index as usize * self.allocator.header().slab_size as usize);

        // We now know the slab index - we can get the slab metadata.
        let slab_meta = unsafe { self.allocator.slab_meta(slab_index).as_mut() };
        let size_class_index = slab_meta.size_class_index as usize;
        let slab_size_class = SIZE_CLASSES[size_class_index];
        let allocation_index_in_slab = (offset_from_slab_start / slab_size_class as usize) as u16;

        // Check if the slab is assigned to this worker.
        if slab_meta.assigned_worker != self.worker_index {
            slab_meta.remote_free_stack.push(
                u32::from(allocation_index_in_slab),
                slab_size_class,
                unsafe { self.allocator.slab(slab_index).as_ptr() },
            );
            return;
        }

        // Local free - push the item back to the slab's free stack.
        self.local_free(slab_index, allocation_index_in_slab);
    }

    /// Drain all remote frees for this worker.
    pub fn drain_remote_frees(&self) {
        for size_index in 0..NUM_SIZE_CLASSES {
            self.drain_remote_frees_for_size_class(size_index as u8);
        }
    }

    pub fn drain_remote_frees_for_size_class(&self, size_class_index: u8) {
        let worker_state = unsafe { self.allocator.worker_state(self.worker_index).as_mut() };
        let partial_head =
            worker_state.partial_slabs_heads[usize::from(size_class_index)].load(Ordering::Acquire);
        for slab_index in unsafe { worker_local_list::iterate(&self.allocator, partial_head) } {
            unsafe {
                for index in self.allocator.drain_remote_frees(slab_index) {
                    self.local_free(slab_index, index as u16);
                }
            }
        }

        let full_head =
            worker_state.full_slabs_heads[usize::from(size_class_index)].load(Ordering::Acquire);
        for slab_index in unsafe { worker_local_list::iterate(&self.allocator, full_head) } {
            unsafe {
                for index in self.allocator.drain_remote_frees(slab_index) {
                    self.local_free(slab_index, index as u16);
                }
            }
        }
    }

    unsafe fn local_free(&self, slab_index: u32, allocation_index_in_slab: u16) {
        let slab_meta = unsafe { self.allocator.slab_meta(slab_index).as_mut() };
        let size_class_index = slab_meta.size_class_index as usize;
        let slab_size = self.allocator.header().slab_size;
        let slab_size_class = SIZE_CLASSES[size_class_index];

        debug_assert!(
            allocation_index_in_slab
                < SlabMeta::capacity(slab_size, slab_meta.size_class_index),
            "allocation index is out of bounds for slab {slab_index}. Index={allocation_index_in_slab}, Capacity={}",
            SlabMeta::capacity(slab_size, slab_meta.size_class_index)
        );
        let free_stack = &mut slab_meta.free_stack;
        free_stack.push(allocation_index_in_slab);

        let free_stack_capacity = slab_size as usize / slab_size_class as usize;

        // If the free stack was empty before this push, we must:
        // 1. Remove the slab from the full list.
        // 2. Push the slab onto the worker's partial list.
        let free_stack_len = free_stack.len();
        let worker_state = unsafe { self.allocator.worker_state(self.worker_index).as_mut() };
        if free_stack_len == 1 {
            worker_local_list::remove_slab_from_list(
                &self.allocator,
                &worker_state.full_slabs_heads[size_class_index],
                slab_index,
            );
            worker_local_list::push_slab_into_list(
                &self.allocator,
                &worker_state.partial_slabs_heads[size_class_index],
                slab_index,
            );
        }
        // If the free stack is now full (i.e. the slab is empty), we must:
        // 1. Remove the slab from the partial list.
        // 2. Push the slab onto the global free stack.
        else if free_stack.len() == free_stack_capacity as u16 {
            worker_local_list::remove_slab_from_list(
                &self.allocator,
                &self
                    .allocator
                    .worker_state(self.worker_index)
                    .as_ref()
                    .partial_slabs_heads[size_class_index],
                slab_index,
            );
            global_free_stack::return_the_slab(&self.allocator, slab_index);
        }
    }
}

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

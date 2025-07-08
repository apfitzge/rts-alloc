use crate::{
    index::NULL_U32,
    raw_allocator::RawAllocator,
    size_classes::{size_class_index, MAX_SIZE, NUM_SIZE_CLASSES, SIZE_CLASSES},
    slab_meta::SlabMeta,
};
use std::{ptr::NonNull, sync::atomic::Ordering};

mod align;
pub mod cache_aligned;
pub mod error;
pub mod free_stack;
mod global_free_stack;
pub mod header;
pub mod index;
pub mod init;
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
            // SAFETY:
            // - The `worker_index` is valid.
            if !unsafe {
                self.allocator
                    .take_free_slab(self.worker_index, size_class_index)
            } {
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

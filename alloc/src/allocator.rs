use crate::cache_aligned::CacheAlignedU32;
use crate::global_free_stack;
use crate::{
    error::Error, header::Header, size_classes::size_class_index, worker_state::WorkerState,
};
use core::mem::offset_of;
use core::ptr::NonNull;

pub struct Allocator {
    header: NonNull<Header>,
    worker_index: u32,
}

impl Allocator {
    /// Creates a new `Allocator` for the given worker index.
    ///
    /// # Safety
    /// - The `header` must point to a valid header of an initialized allocator.
    pub unsafe fn new(header: NonNull<Header>, worker_index: u32) -> Result<Self, Error> {
        // SAFETY: The header is assumed to be valid and initialized.
        if worker_index >= unsafe { header.as_ref() }.num_workers {
            return Err(Error::InvalidWorkerIndex);
        }
        Ok(Allocator {
            header,
            worker_index,
        })
    }

    /// Allocates a block of memory of the given size.
    /// If the size is larger than the maximum size class, returns `None`.
    /// If the allocation fails, returns `None`.
    pub fn allocate(&self, size: u32) -> Option<NonNull<u8>> {
        let size_index = size_class_index(size)?;

        // SAFETY: `size_index` is guaranteed to be valid by `size_class_index`.
        let slab_index = unsafe { self.find_allocatable_slab_index(size_index) }?;
        // SAFETY:
        // - `slab_index` is guaranteed to be valid by `find_allocatable_slab_index`.
        // - `size_index` is guaranteed to be valid by `size_class_index`.
        unsafe { self.allocate_within_slab(slab_index, size_index) }
    }

    /// Try to find a suitable slab for allocation.
    /// If a partial slab assigned to the worker is not found, then try to find
    /// a slab from the global free list.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size classes.
    unsafe fn find_allocatable_slab_index(&self, size_index: usize) -> Option<u32> {
        let worker_state = self.worker_state();

        // SAFETY: `size_index` is guaranteed to be valid by the caller.
        match unsafe { worker_state.partial_head(size_index) } {
            Some(slab_index) => Some(slab_index),
            None => unsafe { self.take_slab(worker_state, size_index) },
        }
    }

    /// Attempt to allocate meomry within a slab.
    /// If the slab is full or the allocation otherwise fails, returns `None`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs
    /// - The `size_index` must be a valid index for the size classes.
    unsafe fn allocate_within_slab(
        &self,
        _slab_index: u32,
        _size_index: usize,
    ) -> Option<NonNull<u8>> {
        todo!()
    }

    /// Attempt to take a slab from the global free list.
    /// If the global free list is empty, returns `None`.
    /// If the slab is successfully taken, it will be marked as assigned to the worker.
    ///
    /// # Safety
    /// - The `worker_state` must be valid and initialized.
    /// - The `size_index` must be a valid index for the size claasses.
    unsafe fn take_slab(&self, worker_state: &WorkerState, size_index: usize) -> Option<u32> {
        let head = self.global_free_stack_head();
        let slab_index = global_free_stack::try_pop_free_slab(head, self.global_free_stack_list())?;
        unsafe { self.mark_slab_as_assigned(slab_index, size_index) };
        // worker_state.push_partial_slab(slab_index, size_index);
        Some(slab_index)
    }

    fn mark_slab_as_assigned(&self, _slab_index: u32, _size_index: usize) {
        todo!()
    }
}

impl Allocator {
    /// Returns a reference to the worker state for the current worker.
    pub fn worker_state(&self) -> &WorkerState {
        // SAFETY: The header is assumed to be valid and initialized.
        let worker_states_ptr = unsafe {
            self.header
                .byte_add(offset_of!(Header, worker_states))
                .cast::<WorkerState>()
        };
        // SAFETY: The worker index is guaranteed to be within bounds by the constructor.
        let worker_state_ptr = unsafe { worker_states_ptr.add(self.worker_index as usize) };

        // SAFETY: The worker state pointer is guaranteed to be valid by the constructor.
        unsafe { worker_state_ptr.as_ref() }
    }

    /// Returns a reference to the global_free_stack head.
    pub fn global_free_stack_head(&self) -> &CacheAlignedU32 {
        // SAFETY: The header is assumed to be valid and initialized.
        let global_free_stack_ptr = unsafe {
            self.header
                .byte_add(offset_of!(Header, global_free_stack_head))
                .cast::<CacheAlignedU32>()
        };

        // SAFETY: The global free stack pointer is guaranteed to be valid by the constructor.
        unsafe { global_free_stack_ptr.as_ref() }
    }

    /// Returns a pointer to the global free stack list.
    pub fn global_free_stack_list(&self) -> NonNull<CacheAlignedU32> {
        todo!()
    }
}

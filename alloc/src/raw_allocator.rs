use crate::index::NULL_U32;
use crate::size_classes::SIZE_CLASSES;
use crate::slab_meta::SlabMeta;
use crate::worker_state::WorkerState;
use crate::{global_free_stack, header::Header, size_classes::NUM_SIZE_CLASSES, worker_local_list};
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

pub struct RawAllocator {
    pub header: NonNull<Header>,
}

impl RawAllocator {
    pub fn header(&self) -> &Header {
        // SAFETY:
        // - The header pointer is guaranteed to be non-null.
        // - The header is properly initialized and aligned.
        unsafe { self.header.as_ref() }
    }

    /// Convert a pointer to an offset from the start of the header.
    ///
    /// # Safety
    /// - `ptr` must be greater than or equal to the `header` pointer.
    pub unsafe fn ptr_to_offset(&self, ptr: NonNull<u8>) -> usize {
        ptr.byte_offset_from_unsigned(self.header)
    }

    /// Convert an offset to a pointer from the start of the header.
    ///
    /// # Safety
    /// - `offset` should be less than the size of the allocator to avoid
    ///   out-of-bounds access.
    pub unsafe fn offset_to_ptr(&self, offset: usize) -> NonNull<u8> {
        self.header.byte_add(offset).cast()
    }

    /// Take a free slab for the worker, for a specific size class.
    ///
    /// # Safety
    /// - The `worker_index` must be less than the number of workers in the allocator.
    pub unsafe fn take_free_slab(&self, worker_index: u32, size_class_index: usize) -> bool {
        let Some(slab_index) = global_free_stack::try_pop_free_slab(self) else {
            return false;
        };

        // No other worker should be touching this workers partial/full lists.
        // No need to do a CAS, since there should not be contention.
        // SAFETY: `worker_index` is valid.
        let worker = unsafe { self.worker_state(worker_index).as_ref() };
        let partial_head = &worker.partial_slabs_heads[size_class_index];
        unsafe {
            worker_local_list::push_slab_into_list(self, partial_head, slab_index);
        }

        // The slab is now part of the worker's partial list.
        // Now set up the slab's metadata to reflect this.
        // SAFETY: `slab_index` is valid.
        let slab_meta = unsafe { self.slab_meta(slab_index).as_mut() };

        // SAFETY: The slab meta must be allocated with enough trailing data for `free_stack`.
        unsafe { slab_meta.assign(self.header().slab_size, worker_index, size_class_index) };

        true
    }

    pub fn clear_worker(&self, worker_index: u32) {
        let worker_state = unsafe { self.worker_state(worker_index).as_mut() };
        for size_index in 0..NUM_SIZE_CLASSES {
            let partial_head = &worker_state.partial_slabs_heads[size_index];
            let mut current_head = partial_head.load(Ordering::Acquire);
            while current_head != NULL_U32 {
                unsafe {
                    worker_local_list::remove_slab_from_list(self, partial_head, current_head);
                    global_free_stack::return_the_slab(self, current_head);
                }
                current_head = partial_head.load(Ordering::Acquire);
            }

            let full_head = &worker_state.full_slabs_heads[size_index];
            let mut current_head = full_head.load(Ordering::Acquire);
            while current_head != NULL_U32 {
                unsafe {
                    worker_local_list::remove_slab_from_list(self, full_head, current_head);
                    global_free_stack::return_the_slab(self, current_head);
                }
                current_head = full_head.load(Ordering::Acquire);
            }
        }
    }

    /// Should only be called by the worker that owns the slab.
    ///
    /// # Safety
    /// - The `slab_index` must be less than the number of slabs in the allocator.
    /// - The `slab_index` must be owned by the worker calling this function.
    pub unsafe fn drain_remote_frees(&self, slab_index: u32) -> impl Iterator<Item = u32> + '_ {
        let slab_meta = unsafe { self.slab_meta(slab_index).as_mut() };
        let slab_item_size = SIZE_CLASSES[usize::from(slab_meta.size_class_index)];
        let slab_ptr = unsafe { self.slab(slab_index).as_ptr() };
        slab_meta.remote_free_stack.drain(slab_item_size, slab_ptr)
    }

    /// Given a slab index, return a pointer to the slab metadata.
    ///
    /// # Safety
    /// - The `slab_index` must be less than the number of slabs in the allocator.
    pub unsafe fn slab_meta(&self, slab_index: u32) -> NonNull<SlabMeta> {
        let header = self.header();
        self.header
            .cast::<u8>()
            .byte_add(header.slab_meta_offset as usize)
            .byte_add(slab_index as usize * header.slab_meta_size as usize)
            .cast()
    }

    /// Given a slab index, return a pointer to the slab memory.
    ///
    /// # Safety
    /// - The `slab_index` must be less than the number of slabs in the allocator.
    pub unsafe fn slab(&self, slab_index: u32) -> NonNull<u8> {
        let header = self.header();
        self.header
            .cast::<u8>()
            .byte_add(header.slab_offset as usize)
            .byte_add((slab_index * header.slab_size) as usize)
    }

    /// Given a worker index, return a pointer to the worker state.
    ///
    /// # Safety
    /// - The `worker_index` must be less than the number of workers in the allocator.
    pub unsafe fn worker_state(&self, worker_index: u32) -> NonNull<WorkerState> {
        let header = self.header();
        let worker_states_ptr = header.worker_states.as_ptr();

        // SAFETY: The `worker_index` must be less than `num_workers`.
        let worker_state_ptr = unsafe { worker_states_ptr.add(worker_index as usize) };

        // SAFETY: The `worker_state_ptr` is not null.
        NonNull::new(worker_state_ptr.cast_mut()).expect("Worker state pointer should not be null")
    }
}

use crate::free_list_element::FreeListElement;
use crate::free_stack::FreeStack;
use crate::global_free_list::GlobalFreeList;
use crate::header::{self, WorkerLocalListHeads, WorkerLocalListPartialFullHeads};
use crate::remote_free_list::RemoteFreeList;
use crate::size_classes::{size_class, NUM_SIZE_CLASSES};
use crate::slab_meta::SlabMeta;
use crate::worker_local_list::WorkerLocalList;
use crate::{error::Error, header::Header, size_classes::size_class_index};
use core::mem::offset_of;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;
use std::path::Path;

pub struct Allocator {
    header: NonNull<Header>,
    worker_index: u32,
}

impl Allocator {
    /// Create a new `Allocator` with new file and given parameters.
    pub fn create(
        path: impl AsRef<Path>,
        file_size: usize,
        num_workers: u32,
        slab_size: u32,
        delete_existing: bool,
        worker_index: u32,
    ) -> Result<Self, Error> {
        if worker_index >= num_workers {
            return Err(Error::InvalidWorkerIndex);
        }
        let header = crate::init::create(path, file_size, num_workers, slab_size, delete_existing)?;

        // SAFETY: The header is guaranteed to be valid and initialized.
        unsafe { Allocator::new(header, worker_index) }
    }

    /// Join an existing allocator at the given path.
    pub fn join(path: impl AsRef<Path>, worker_index: u32) -> Result<Self, Error> {
        let header = crate::init::join(path)?;

        // Check if the worker index is valid.
        // SAFETY: The header is guaranteed to be valid and initialized.
        if worker_index >= unsafe { header.as_ref() }.num_workers {
            return Err(Error::InvalidWorkerIndex);
        }

        // SAFETY: The header is guaranteed to be valid and initialized.
        unsafe { Allocator::new(header, worker_index) }
    }

    /// Creates a new `Allocator` for the given worker index.
    ///
    /// # Safety
    /// - The `header` must point to a valid header of an initialized allocator.
    unsafe fn new(header: NonNull<Header>, worker_index: u32) -> Result<Self, Error> {
        // SAFETY: The header is assumed to be valid and initialized.
        if worker_index >= unsafe { header.as_ref() }.num_workers {
            return Err(Error::InvalidWorkerIndex);
        }
        Ok(Allocator {
            header,
            worker_index,
        })
    }
}

impl Allocator {
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
        // SAFETY: `size_index` is guaranteed to be valid by the caller.
        unsafe { self.worker_local_list_partial(size_index) }
            .head()
            .or_else(|| self.take_slab(size_index))
    }

    /// Attempt to allocate meomry within a slab.
    /// If the slab is full or the allocation otherwise fails, returns `None`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs
    /// - The `size_index` must be a valid index for the size classes.
    unsafe fn allocate_within_slab(
        &self,
        slab_index: u32,
        size_index: usize,
    ) -> Option<NonNull<u8>> {
        // SAFETY: The slab index is guaranteed to be valid by the caller.
        let free_stack = unsafe { self.slab_free_stack(slab_index) };
        let maybe_index_within_slab = free_stack.pop();

        // If the slab is empty - remove it from the worker's partial list,
        // and move it to the worker's full list.
        if free_stack.is_empty() {
            // SAFETY:
            // - The `slab_index` is guaranteed to be valid by the caller.
            // - The `size_index` is guaranteed to be valid by the caller.
            unsafe {
                self.worker_local_list_partial(size_index)
                    .remove(slab_index);
            }
            // SAFETY:
            // - The `slab_index` is guaranteed to be valid by the caller.
            // - The `size_index` is guaranteed to be valid by the caller.
            unsafe {
                self.worker_local_list_full(size_index).push(slab_index);
            }
        }

        maybe_index_within_slab.map(|index_within_slab| {
            // SAFETY: The `slab_index` is guaranteed to be valid by the caller.
            let slab = unsafe { self.slab(slab_index) };
            // SAFETY: The `size_index` is guaranteed to be valid by the caller.
            let size = unsafe { size_class(size_index) };
            slab.byte_add(index_within_slab as usize * size as usize)
        })
    }

    /// Attempt to take a slab from the global free list.
    /// If the global free list is empty, returns `None`.
    /// If the slab is successfully taken, it will be marked as assigned to the worker.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size claasses.
    unsafe fn take_slab(&self, size_index: usize) -> Option<u32> {
        let slab_index = self.global_free_list().pop()?;

        // SAFETY: The slab index is guaranteed to be valid by `pop`.
        unsafe { self.slab_meta(slab_index).as_ref() }.assign(self.worker_index, size_index);
        // SAFETY:
        // - The slab index is guaranteed to be valid by `pop`.
        // - The size index is guaranteed to be valid by the caller.
        unsafe {
            let slab_capacity = self.header.as_ref().slab_size / size_class(size_index);
            self.slab_free_stack(slab_index).reset(slab_capacity as u16);
        };
        // SAFETY: The size index is guaranteed to be valid by caller.
        let mut worker_local_list = unsafe { self.worker_local_list_partial(size_index) };
        // SAFETY: The slab index is guaranteed to be valid by `pop`.
        unsafe { worker_local_list.push(slab_index) };
        Some(slab_index)
    }
}

impl Allocator {
    /// Free a block of memory previously allocated by this allocator.
    ///
    /// # Safety
    /// - The `ptr` must be a valid pointer to a block of memory allocated by this allocator.
    /// - The `ptr` must not have been freed before.
    pub unsafe fn free(&self, ptr: NonNull<u8>) {
        // SAFETY: The pointer is assumed to be valid and allocated by this allocator.
        let offset = unsafe { self.offset(ptr) };
        let allocation_indexes = self.find_allocation_indexes(offset);

        // Check if the slab is assigned to this worker.
        if self.worker_index
            == unsafe { self.slab_meta(allocation_indexes.slab_index).as_ref() }
                .assigned_worker
                .load(Ordering::Acquire)
        {
            self.local_free(allocation_indexes);
        } else {
            self.remote_free(allocation_indexes);
        }
    }

    fn local_free(&self, allocation_indexes: AllocationIndexes) {
        // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
        let (was_full, is_empty) = unsafe {
            let free_stack = self.slab_free_stack(allocation_indexes.slab_index);
            let was_full = free_stack.is_empty();
            free_stack.push(allocation_indexes.index_within_slab);
            // Names confusing:
            // - When the **free** stack is empty, the slab is full of allocations.
            // - When the **free** stack is full, the slab has no allocations available.
            (was_full, free_stack.is_full())
        };

        match (was_full, is_empty) {
            (true, true) => {
                // The slab was full and is now empty - this cannot happen unless the slab
                // size is equal to the size class.
                unreachable!("slab can only contain one allocation - this is not allowed");
            }
            (true, false) => {
                // The slab was full and is now partially full. It must be moved
                // from the worker's full list to the worker's partial list.
                // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
                unsafe {
                    self.worker_local_list_full(allocation_indexes.slab_index as usize)
                        .remove(allocation_indexes.slab_index);
                }
                // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
                unsafe {
                    self.worker_local_list_partial(allocation_indexes.slab_index as usize)
                        .push(allocation_indexes.slab_index);
                }
            }
            (false, true) => {
                // The slab was partially full and is now empty.
                // It must be moved from the worker's partial list to the global free list.
                // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
                unsafe {
                    self.worker_local_list_partial(allocation_indexes.slab_index as usize)
                        .remove(allocation_indexes.slab_index);
                }
                // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
                unsafe {
                    self.global_free_list().push(allocation_indexes.slab_index);
                }
            }
            (false, false) => {
                // The slab was partially full and is still partially full.
                // No action is needed, just return.
            }
        }
    }

    fn remote_free(&self, allocation_indexes: AllocationIndexes) {
        // SAFETY: The allocation indexes are guaranteed to be valid by the caller.
        unsafe {
            self.remote_free_list(allocation_indexes.slab_index)
                .push(allocation_indexes.index_within_slab);
        }
    }

    /// Find the offset given a pointer.
    ///
    /// # Safety
    /// - The `ptr` must be a valid pointer in the allocator's address space.
    unsafe fn offset(&self, ptr: NonNull<u8>) -> u32 {
        ptr.byte_offset_from_unsigned(self.header) as u32
    }

    /// Find the slab index and index within the slab for a given offset.
    fn find_allocation_indexes(&self, offset: u32) -> AllocationIndexes {
        let (slab_index, offset_within_slab) = {
            // SAFETY: The header is assumed to be valid and initialized.
            let header = unsafe { self.header.as_ref() };
            debug_assert!(offset >= header.slabs_offset);
            let offset_from_slab_start = offset.wrapping_sub(header.slabs_offset);
            let slab_index = offset_from_slab_start / header.slab_size;
            debug_assert!(slab_index < header.num_slabs, "slab index out of bounds");

            // SAFETY: The slab size is guaranteed to be a power of 2, for a valid header.
            let offset_within_slab =
                unsafe { Self::offset_within_slab(header.slab_size, offset_from_slab_start) };

            (slab_index, offset_within_slab)
        };

        let index_within_slab = {
            // SAFETY: The slab index is guaranteed to be valid by the above calculations.
            let size_class_index = unsafe { self.slab_meta(slab_index).as_ref() }
                .size_class_index
                .load(Ordering::Acquire);
            // SAFETY: The size class index is guaranteed to be valid by valid slab meta.
            let size_class = unsafe { size_class(size_class_index) };
            (offset_within_slab / size_class) as u16
        };

        AllocationIndexes {
            slab_index,
            index_within_slab,
        }
    }

    /// Return offset within a slab.
    ///
    /// # Safety
    /// - The `slab_size` must be a power of 2.
    const unsafe fn offset_within_slab(slab_size: u32, offset_from_slab_start: u32) -> u32 {
        debug_assert!(slab_size.is_power_of_two());
        offset_from_slab_start & (slab_size - 1)
    }
}

impl Allocator {
    /// Frees all items in remote free lists.
    pub fn clean_remote_free_lists(&self) {
        // Do partial slabs before full slabs, because the act of freeing within
        // the full slabs may move them to partial slabs list, which would lead
        // to us double-iterating.
        for size_index in 0..NUM_SIZE_CLASSES {
            // SAFETY: The size index is guaranteed to be valid by the loop.
            let worker_local_list = unsafe { self.worker_local_list_partial(size_index) };
            self.clean_remote_free_lists_for_list(worker_local_list);

            // SAFETY: The size index is guaranteed to be valid by the loop.
            let worker_local_list = unsafe { self.worker_local_list_full(size_index) };
            self.clean_remote_free_lists_for_list(worker_local_list);
        }
    }

    /// Frees all items in the remote free list for the given worker local list.
    fn clean_remote_free_lists_for_list(&self, worker_local_list: WorkerLocalList) {
        for slab_index in worker_local_list.iterate() {
            // SAFETY: The slab index is guaranteed to be valid by the iterator.
            let remote_free_list = unsafe { self.remote_free_list(slab_index) };
            for index_within_slab in remote_free_list.drain() {
                self.local_free(AllocationIndexes {
                    slab_index,
                    index_within_slab,
                })
            }
        }
    }
}

impl Allocator {
    /// Returns a pointer to the free list elements in allocator.
    pub fn free_list_elements(&self) -> NonNull<FreeListElement> {
        // SAFETY: The header is assumed to be valid and initialized.
        let offset = unsafe { self.header.as_ref() }.free_list_elements_offset;
        // SAFETY: The header is guaranteed to be valid and initialized.
        unsafe { self.header.byte_add(offset as usize) }.cast()
    }

    /// Returns a `GlobalFreeList` to interact with the global free list.
    pub fn global_free_list(&self) -> GlobalFreeList {
        // SAFETY: The header is assumed to be valid and initialized.
        let head = &unsafe { self.header.as_ref() }.global_free_list_head;
        let list = self.free_list_elements();
        // SAFETY:
        // - `head` is a valid reference to the global free list head.
        // - `list` is guaranteed to be valid wtih sufficient capacity.
        unsafe { GlobalFreeList::new(head, list) }
    }

    /// Returns a `WorkerLocalList` for the current worker to interact with its
    /// local free list of partially full slabs.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size classes.
    pub unsafe fn worker_local_list_partial(&self, size_index: usize) -> WorkerLocalList {
        let head = {
            // SAFETY: The header is assumed to be valid and initialized.
            let all_workers_heads = unsafe {
                self.header
                    .byte_add(offset_of!(Header, worker_local_list_heads))
                    .cast::<WorkerLocalListHeads>()
            };
            // SAFETY: The worker index is guaranteed to be valid by the constructor.
            let worker_heads = unsafe { all_workers_heads.add(self.worker_index as usize) };
            // SAFETY: The size index is guaranteed to be valid by the caller.
            &mut unsafe {
                worker_heads
                    .cast::<WorkerLocalListPartialFullHeads>()
                    .add(size_index)
                    .as_mut()
            }
            .partial
        };
        let list = self.free_list_elements();
        // SAFETY:
        // - `head` is a valid reference to the worker's local list head.
        // - `list` is guaranteed to be valid with sufficient capacity.
        unsafe { WorkerLocalList::new(head, list) }
    }

    /// Returns a `WorkerLocalList` for the current worker to interact with its
    /// local free list of full slabs.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size classes.
    pub unsafe fn worker_local_list_full(&self, size_index: usize) -> WorkerLocalList {
        let head = {
            // SAFETY: The header is assumed to be valid and initialized.
            let all_workers_heads = unsafe {
                self.header
                    .byte_add(offset_of!(Header, worker_local_list_heads))
                    .cast::<WorkerLocalListHeads>()
            };
            // SAFETY: The worker index is guaranteed to be valid by the constructor.
            let worker_heads = unsafe { all_workers_heads.add(self.worker_index as usize) };
            // SAFETY: The size index is guaranteed to be valid by the caller.
            &mut unsafe {
                worker_heads
                    .cast::<WorkerLocalListPartialFullHeads>()
                    .add(size_index)
                    .as_mut()
            }
            .full
        };
        let list = self.free_list_elements();
        // SAFETY:
        // - `head` is a valid reference to the worker's local list head.
        // - `list` is guaranteed to be valid with sufficient capacity.
        unsafe { WorkerLocalList::new(head, list) }
    }

    /// Returns an instance of `RemoteFreeList` for the given slab.
    ///
    /// # Safety
    /// - `slab_index` must be a valid slab index.
    pub unsafe fn remote_free_list(&self, slab_index: u32) -> RemoteFreeList {
        let (head, slab_item_size) = {
            // SAFETY: The slab index is guaranteed to be valid by the caller.
            let slab_meta = unsafe { self.slab_meta(slab_index).as_ref() };
            // SAFETY: The slab meta is guaranteed to be valid by the caller.
            let size_class =
                unsafe { size_class(slab_meta.size_class_index.load(Ordering::Acquire)) };
            (&slab_meta.remote_free_stack_head, size_class)
        };
        // SAFETY: The slab index is guaranteed to be valid by the caller.
        let slab = unsafe { self.slab(slab_index) };

        // SAFETY:
        // - `slab_item_size` must be a valid size AND currently assigned to the slab.
        // - `head` must be a valid reference to a `CacheAlignedU16
        //   that is the head of the remote free list.
        // - `slab` must be a valid pointer to the beginning of the slab.
        unsafe { RemoteFreeList::new(slab_item_size, head, slab) }
    }

    /// Returns a pointer to the slab meta for the given slab index.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs.
    pub unsafe fn slab_meta(&self, slab_index: u32) -> NonNull<SlabMeta> {
        // SAFETY: The header is assumed to be valid and initialized.
        let offset = unsafe { self.header.as_ref() }.slab_shared_meta_offset;
        // SAFETY: The header is guaranteed to be valid and initialized.
        let slab_metas = unsafe { self.header.byte_add(offset as usize).cast::<SlabMeta>() };
        // SAFETY: The `slab_index` is guaranteed to be valid by the caller.
        unsafe { slab_metas.add(slab_index as usize) }
    }

    /// Return a mutable reference to a free stack for the given slab index.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs.
    pub unsafe fn slab_free_stack(&self, slab_index: u32) -> &FreeStack {
        let (slab_size, offset) = {
            // SAFETY: The header is assumed to be valid and initialized.
            let header = unsafe { self.header.as_ref() };
            (header.slab_size, header.slab_free_stacks_offset)
        };
        let free_stack_size = header::layout::single_free_stack_size(slab_size);

        // SAFETY: The header is guaranteed to be valid and initialized.
        // The free stacks are laid out sequentially after the slab meta.
        self.header
            .byte_add(offset as usize)
            .byte_add(slab_index as usize * free_stack_size)
            .cast()
            .as_ref()
    }

    /// Return a pointer to a slab.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs.
    pub unsafe fn slab(&self, slab_index: u32) -> NonNull<u8> {
        let (slab_size, offset) = {
            // SAFETY: The header is assumed to be valid and initialized.
            let header = unsafe { self.header.as_ref() };
            (header.slab_size, header.slabs_offset)
        };
        // SAFETY: The header is guaranteed to be valid and initialized.
        // The slabs are laid out sequentially after the free stacks.
        unsafe {
            self.header
                .byte_add(offset as usize)
                .byte_add(slab_index as usize * slab_size as usize)
                .cast()
        }
    }
}

struct AllocationIndexes {
    slab_index: u32,
    index_within_slab: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        header::layout,
        init::initialize,
        size_classes::{MAX_SIZE, SIZE_CLASSES},
    };

    const TEST_BUFFER_SIZE: usize = 64 * 1024 * 1024; // 1 MiB

    fn initialize_for_test(
        buffer: &mut [u8],
        slab_size: u32,
        num_workers: u32,
        worker_index: u32,
    ) -> Allocator {
        let file_size = buffer.len();

        let layout = layout::offsets(file_size, slab_size, num_workers);

        let header = NonNull::new(buffer.as_mut_ptr() as *mut Header).unwrap();
        // SAFETY: The header is valid for any byte pattern, and we are initializing it with the
        //         allocator.
        unsafe {
            initialize::allocator(header, slab_size, num_workers, layout);
        }

        // SAFETY: The header/allocator memory is initialized.
        unsafe { Allocator::new(header, worker_index) }.unwrap()
    }

    #[test]
    fn test_allocator() {
        let mut buffer = vec![0u8; TEST_BUFFER_SIZE];
        let slab_size = 65536; // 64 KiB
        let num_workers = 4;
        let worker_index = 0;
        let allocator = initialize_for_test(&mut buffer, slab_size, num_workers, worker_index);

        let mut allocations = vec![];

        for size_class in SIZE_CLASSES[..NUM_SIZE_CLASSES - 1].iter() {
            for size in [size_class - 1, *size_class, size_class + 1] {
                allocations.push(allocator.allocate(size).unwrap());
            }
        }
        for size in [MAX_SIZE - 1, MAX_SIZE] {
            allocations.push(allocator.allocate(size).unwrap());
        }
        assert!(allocator.allocate(MAX_SIZE + 1).is_none());

        // The worker should have local lists for all size classes.
        for size_index in 0..NUM_SIZE_CLASSES {
            // SAFETY: The size index is guaranteed to be valid by the loop.
            let worker_local_list = unsafe { allocator.worker_local_list_partial(size_index) };
            assert!(worker_local_list.head().is_some());
        }

        for ptr in allocations {
            // SAFETY: ptr is valid allocation from the allocator.
            unsafe {
                allocator.free(ptr);
            }
        }

        // The worker local lists should be empty after freeing.
        for size_index in 0..NUM_SIZE_CLASSES {
            // SAFETY: The size index is guaranteed to be valid by the loop.
            let worker_local_list = unsafe { allocator.worker_local_list_partial(size_index) };
            assert_eq!(worker_local_list.head(), None);
        }
    }
}

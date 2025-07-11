use crate::free_list_element::FreeListElement;
use crate::free_stack::FreeStack;
use crate::global_free_list::GlobalFreeList;
use crate::header::{self, WorkerLocalListHeads};
use crate::size_classes::size_class;
use crate::slab_meta::SlabMeta;
use crate::worker_local_list::WorkerLocalList;
use crate::{error::Error, header::Header, size_classes::size_class_index};
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
        // SAFETY: `size_index` is guaranteed to be valid by the caller.
        unsafe { self.worker_local_list(size_index) }
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

        // If the slab is empty - remove it from the worker's local list.
        if free_stack.is_empty() {
            // SAFETY:
            // - The `slab_index` is guaranteed to be valid by the caller.
            // - The `size_index` is guaranteed to be valid by the caller.
            unsafe {
                self.worker_local_list(size_index).remove(slab_index);
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
        // SAFETY: The size index is guaranteed to be valid by caller.
        let mut worker_local_list = unsafe { self.worker_local_list(size_index) };
        // SAFETY: The slab index is guaranteed to be valid by `pop`.
        unsafe { worker_local_list.push(slab_index) };
        Some(slab_index)
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
    /// local free list.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size classes.
    pub unsafe fn worker_local_list(&self, size_index: usize) -> WorkerLocalList {
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
            unsafe { worker_heads.cast::<u32>().add(size_index).as_mut() }
        };
        let list = self.free_list_elements();
        // SAFETY:
        // - `head` is a valid reference to the worker's local list head.
        // - `list` is guaranteed to be valid with sufficient capacity.
        unsafe { WorkerLocalList::new(head, list) }
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

    /// Return a reference to a free stack for the given slab index.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index for the slabs.
    pub unsafe fn slab_free_stack(&self, slab_index: u32) -> &FreeStack {
        let (slab_size, offset) = {
            // SAFETY: The header is assumed to be valid and initialized.
            let header = unsafe { self.header.as_ref() };
            (header.slab_size, header.slab_free_stacks_offset)
        };
        let free_stack_size = header::layout::free_stacks_size(slab_size);

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

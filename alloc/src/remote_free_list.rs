use crate::cache_aligned::CacheAlignedU16;
use core::ptr::NonNull;
use core::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;

/// A shared remote free list for a slab.
/// Intrusive linked list where each element is used as either a
/// element storing data or as a link to the next element.
/// Lock-free and thread/process safe.
pub struct RemoteFreeList<'a> {
    /// Size of each element in the slab.
    slab_item_size: u32,
    /// The head of the remote free list.
    head: &'a CacheAlignedU16,
    /// Pointer to the beginnning of the slab.
    slab: NonNull<u8>,
}

impl<'a> RemoteFreeList<'a> {
    /// Creates a new remote free list.
    ///
    /// # Safety
    /// - `slab_item_size` must be a valid size AND currently assigned to the slab.
    /// - `head` must be a valid reference to a `CacheAlignedU32` that is the head
    ///   of the remote free list.
    /// - `slab` must be a valid pointer to the beginning of the slab.
    pub unsafe fn new(slab_item_size: u32, head: &'a CacheAlignedU16, slab: NonNull<u8>) -> Self {
        Self {
            slab_item_size,
            head,
            slab,
        }
    }

    /// Pushes an item to the remote free list.
    ///
    /// # Safety
    /// - The `index` must be a valid index within the slab.
    /// - The `index` must not already be in the remote free list.
    pub unsafe fn push(&self, index: u16) {
        // SAFETY: The index is guaranteed to be valid by the caller.
        let item: &AtomicU16 = unsafe {
            self.slab
                .byte_add(index as usize * self.slab_item_size as usize)
                .cast()
                .as_ref()
        };

        loop {
            // Read the current head of the remote free list.
            let current_head = self.head.load(Ordering::Acquire);
            // Set the next pointer of the item to the current head.
            item.store(current_head, Ordering::Release);
            // Attempt to set the head to the item.
            if self
                .head
                .compare_exchange(current_head, index, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Successfully pushed the item to the remote free list.
                break;
            }
        }
    }
}

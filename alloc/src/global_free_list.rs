use crate::cache_aligned::CacheAlignedU32;
use core::sync::atomic::{AtomicU32, Ordering};

/// A singly linked-list that tracks slabs not assigned to any worker.
#[repr(C)]
pub struct GlobalFreeList {
    head: CacheAlignedU32,
    list: [AtomicU32; 0],
}

impl GlobalFreeList {
    /// Return the minimum byte size of the global free list with
    /// given `capacity`.
    pub const fn byte_size(capacity: usize) -> usize {
        core::mem::size_of::<GlobalFreeList>() + capacity * core::mem::size_of::<AtomicU32>()
    }

    /// Pushes `slab_index` onto the head of the global free list.
    ///
    /// # Safety
    /// - `slab_index` must be a valid index into the `list`.
    pub fn push(&self, slab_index: u32) {
        // SAFETY: The `slab_index` is assumed to be a valid index into the `list`.
        let next_head_ref = unsafe { self.get_unchecked(slab_index) };
        loop {
            let current_head = self.head.load(Ordering::Acquire);
            next_head_ref.store(current_head, Ordering::Release);
            if self
                .head
                .compare_exchange(
                    current_head,
                    slab_index,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return; // Successfully pushed the slab index onto the list
            }
        }
    }

    /// Get reference to a specific slab indexes `AtomicU32`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index into the `list`.
    unsafe fn get_unchecked(&self, slab_index: u32) -> &AtomicU32 {
        &*self.list.as_ptr().add(slab_index as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_global_free_list() {
        const LIST_CAPACITY: usize = 1024;
        let mut buffer = [0u8; GlobalFreeList::byte_size(LIST_CAPACITY)];

        // SAFETY: The buffer is large enough to hold a GlobalFreeList with the specified capacity.
        let global_free_list = unsafe {
            buffer
                .as_mut_ptr()
                .cast::<GlobalFreeList>()
                .as_mut()
                .unwrap()
        };

        // Push a slab index onto the global free list.
        global_free_list.push(0);
    }
}

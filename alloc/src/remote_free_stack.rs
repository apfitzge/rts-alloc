use crate::{cache_aligned::CacheAlignedU32, NULL};
use std::sync::atomic::Ordering;

/// The remote free stack is a lock-free stack that allows pushing and popping
/// of indices from an intrusive linked list.
/// It is intended to be used in a MPSC manner, where multiple producers are
/// pushing freed indices and a single consumer is popping them.
#[repr(C)]
pub struct RemoteFreeStack {
    head: CacheAlignedU32,
}

impl RemoteFreeStack {
    /// Sets up the remote free stack with no items in the stack.
    pub fn reset(&mut self) {
        self.head.store(NULL, Ordering::Release);
    }

    /// Swaps the head of the stack with NULL, effectively clearing the stack.
    /// Returns an iterator over the indices that were in the stack.
    ///
    /// # Safety
    /// - The `slab_item_size` must be a valid size for the items in the slab -
    ///   i.e. one of the `SIZE_CLASSES`.
    /// - The `slab_ptr` must point to a valid slab memory region.
    pub unsafe fn drain(
        &self,
        slab_item_size: u32,
        slab_ptr: *const u8,
    ) -> impl Iterator<Item = u32> + '_ {
        let current = self.head.swap(NULL, Ordering::AcqRel);
        RemoteFreeStackIter {
            current,
            slab_item_size,
            slab_ptr,
        }
    }

    /// Pushes an item index onto the remote free stack.
    ///
    /// # Safety
    /// - The `item_index` must be a valid index within the slab allocation:
    ///   - i.e. `0 <= item < slab_size / slab_item_size`.
    /// - The `slab_item_size` must be a valid size for the items in
    ///   the slab - i.e. one of the `SIZE_CLASSES`.
    /// - The `slab_ptr` must point to a valid slab memory region.
    pub unsafe fn push(&self, item_index: u32, slab_item_size: u32, slab_ptr: *mut u8) {
        // SAFETY: The `item_index` is a valid index within the slab allocation.
        let pushed_slot = slab_ptr
            .byte_add(item_index as usize * slab_item_size as usize)
            .cast::<u32>();
        loop {
            // Write the current head to the slab at the item index.
            let current = self.head.load(Ordering::Acquire);

            // SAFETY: The `pushed_slot` is a valid pointer to a u32.
            unsafe {
                pushed_slot.write(current);
            }

            // Attempt to set the head to the item index using CAS.
            if self
                .head
                .compare_exchange_weak(current, item_index, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Successfully pushed the item onto the stack.
                return;
            }
        }
    }
}

pub struct RemoteFreeStackIter {
    current: u32,
    slab_item_size: u32,
    slab_ptr: *const u8,
}

impl Iterator for RemoteFreeStackIter {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == NULL {
            return None;
        }

        // Read the current value and move to the next.
        let current = self.current;
        let next = unsafe {
            let next_ptr = self
                .slab_ptr
                .add(self.current as usize * self.slab_item_size as usize);
            // Treat the next slab slot as an u32 (intrusive linked list) - read the value as next.
            (next_ptr as *const u32).read()
        };

        // Update the current to the next value.
        self.current = next;

        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_aligned::CacheAligned;

    #[test]
    fn test_remote_free_stack_push_pop() {
        const SLAB_SIZE: u32 = 64;
        const NUM_SLABS: u32 = 32;
        let mut stack = [[0u8; SLAB_SIZE as usize]; NUM_SLABS as usize];
        let stack_ptr = stack.as_mut_ptr().cast::<u8>();

        let remote_free_stack = RemoteFreeStack {
            head: CacheAligned(NULL.into()),
        };

        // Push until full
        for index in 0..NUM_SLABS {
            unsafe {
                remote_free_stack.push(index, SLAB_SIZE, stack_ptr);
            }
        }

        // Drain the stack and collect the indices.
        let drained_indices: Vec<u32> =
            unsafe { remote_free_stack.drain(SLAB_SIZE, stack_ptr).collect() };

        let expected: Vec<u32> = (0..NUM_SLABS).rev().collect();
        assert_eq!(drained_indices, expected);
    }

    #[test]
    fn test_remote_free_stack_swapped_head() {
        const SLAB_SIZE: u32 = 64;
        const NUM_SLABS: u32 = 32;
        let mut stack = [[0u8; SLAB_SIZE as usize]; NUM_SLABS as usize];
        let stack_ptr = stack.as_mut_ptr().cast::<u8>();

        let remote_free_stack = RemoteFreeStack {
            head: CacheAligned(NULL.into()),
        };

        // Push first half of the stack.
        for index in 0..NUM_SLABS / 2 {
            unsafe {
                remote_free_stack.push(index, SLAB_SIZE, stack_ptr);
            }
        }

        // Get the iterator for draining the stack, but do not iterate over it yet.
        let iter = unsafe { remote_free_stack.drain(SLAB_SIZE, stack_ptr) };

        // Push the second half of the stack.
        for index in NUM_SLABS / 2..NUM_SLABS {
            unsafe {
                remote_free_stack.push(index, SLAB_SIZE, stack_ptr);
            }
        }

        // Iterating over the drain should only yield the first half of the stack.
        let drained_indices: Vec<u32> = iter.collect();
        let expected: Vec<u32> = (0..NUM_SLABS / 2).rev().collect();
        assert_eq!(drained_indices, expected);

        // Draining again should yield the second half of the stack.
        let drained_indices: Vec<u32> =
            unsafe { remote_free_stack.drain(SLAB_SIZE, stack_ptr).collect() };
        let expected: Vec<u32> = (NUM_SLABS / 2..NUM_SLABS).rev().collect();
        assert_eq!(drained_indices, expected);
    }
}

//! Includes all functions related to modifying the global free stack.
//! These are internal functions that should be used by the allocator only.

use crate::{RawAllocator, NULL_U32};
use std::sync::atomic::Ordering;

/// Push a slab index onto the global free stack.
pub unsafe fn return_the_slab(allocator: &RawAllocator, index: u32) {
    loop {
        let current_head = allocator.header().global_free_stack.load(Ordering::Acquire);
        let slab_meta = unsafe { allocator.slab_meta(index).as_mut() };
        slab_meta.assigned_worker = NULL_U32; // mark the slab as unassigned.
        slab_meta.prev = NULL_U32; // not used in global free stack - clear for safety.
        slab_meta.next = current_head; // link the slab to the global free stack.
        if allocator
            .header()
            .global_free_stack
            .compare_exchange(current_head, index, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            break;
        }
    }
}

/// Try to pop a free slab index from the global free stack.
/// Returns `None` if the stack is empty.
pub fn try_pop_free_slab(allocator: &RawAllocator) -> Option<u32> {
    let header = allocator.header();
    loop {
        let current_head = header.global_free_stack.load(Ordering::Acquire);
        if current_head == NULL_U32 {
            return None;
        }

        let next_slab = unsafe { allocator.slab_meta(current_head).as_ref().next };
        if header
            .global_free_stack
            .compare_exchange(current_head, next_slab, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some(current_head);
        }
    }
}

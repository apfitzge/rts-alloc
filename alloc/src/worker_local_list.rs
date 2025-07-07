//! Includes all functions related to the worker-local free lists.
//! These are internal functions that should be used by the allocator only,
//! and more specifically should only be called by the assigned worker.

use crate::{cache_aligned::CacheAlignedU32, RawAllocator, NULL_U32};
use std::sync::atomic::Ordering;

/// Push `slab_index` onto a worker's list given the head.
pub unsafe fn push_slab_into_list(
    allocator: &RawAllocator,
    head: &CacheAlignedU32,
    slab_index: u32,
) {
    let current_head = head.load(Ordering::Relaxed);
    let slab_meta = unsafe { allocator.slab_meta(slab_index).as_mut() };

    slab_meta.prev = NULL_U32; // no previous slab in the list.
    slab_meta.next = current_head; // link the slab to the list.

    if current_head != NULL_U32 {
        // If the current head is not NULL, we need to link it to the new slab.
        let current_head_meta = unsafe { allocator.slab_meta(current_head).as_mut() };
        current_head_meta.prev = slab_index; // link the current head to the new slab.
    }

    head.store(slab_index, Ordering::Relaxed);
}

/// Remove a slab from a specific list, given the head.
pub unsafe fn remove_slab_from_list(
    allocator: &RawAllocator,
    head: &CacheAlignedU32,
    slab_index: u32,
) {
    let current_head = head.load(Ordering::Relaxed);
    debug_assert_ne!(
        current_head, NULL_U32,
        "List head should not be NULL, since we're removing a slab"
    );

    if current_head == slab_index {
        // Link head to the next slab.
        let slab_meta = unsafe { allocator.slab_meta(slab_index).as_mut() };
        let next_slab_index = slab_meta.next;
        head.store(next_slab_index, Ordering::Relaxed);
    }
    unlink_slab_from_list(allocator, slab_index);
}

/// Iterate over the slabs in a worker's list.
pub unsafe fn iterate(
    allocator: &RawAllocator,
    mut current_head: u32,
) -> impl Iterator<Item = u32> + '_ {
    std::iter::from_fn(move || {
        if current_head == NULL_U32 {
            return None;
        }
        let slab_meta = unsafe { allocator.slab_meta(current_head).as_ref() };
        let slab_index = current_head;
        current_head = slab_meta.next;
        Some(slab_index)
    })
}

/// Unlink slab from the worker-local free list (double linked list).
unsafe fn unlink_slab_from_list(allocator: &RawAllocator, slab_index: u32) {
    let slab_meta = unsafe { allocator.slab_meta(slab_index).as_mut() };
    let prev_slab_index = slab_meta.prev;
    let next_slab_index = slab_meta.next;

    // If the prev_slab_index is set, we need to link prev to next.
    if prev_slab_index != NULL_U32 {
        let prev_slab_meta = unsafe { allocator.slab_meta(prev_slab_index).as_mut() };
        prev_slab_meta.next = next_slab_index; // link previous slab to next slab.
    }

    // If the next_slab_index is set, we need to link next to prev.
    if next_slab_index != NULL_U32 {
        let next_slab_meta = unsafe { allocator.slab_meta(next_slab_index).as_mut() };
        next_slab_meta.prev = prev_slab_index; // link next slab to previous slab
    }
}

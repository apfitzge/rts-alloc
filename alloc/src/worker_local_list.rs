use crate::{free_list_element::FreeListElement, index::NULL_U32};
use core::ptr::NonNull;
use std::sync::atomic::Ordering;

/// A doubly linked-list that tracks slabs assigned to a worker.
/// This list is not safe to use concurrently across processes,
/// and should only be accessed by the associated worker.
pub struct WorkerLocalList<'a> {
    head: &'a mut u32,
    list: NonNull<FreeListElement>,
}

impl<'a> WorkerLocalList<'a> {
    /// Creates a new `WorkerLocalList` with the given `head` and `list`.
    ///
    /// # Safety
    /// - `head` must be a valid index into the `list` or NULL_U32.
    /// - `list` must be a valid pointer to an array of `FreeListElement` with sufficient capacity.
    pub unsafe fn new(head: &'a mut u32, list: NonNull<FreeListElement>) -> Self {
        Self { head, list }
    }

    /// Initializes the worker local list as empty with given `capacity`.
    /// # Safety
    /// - `capacity` must be a valid size for the `list`.
    pub unsafe fn initialize_as_empty(&mut self, capacity: u32) {
        *self.head = NULL_U32;
        for slab_index in 0..capacity {
            // SAFETY: The `slab_index` is a valid index into the `list
            unsafe {
                let node = self.get_unchecked(slab_index);
                node.worker_local_prev.store(NULL_U32, Ordering::Release);
                node.worker_local_next.store(NULL_U32, Ordering::Release);
            }
        }
    }

    /// Pushes `slab_index` onto the head of the worker local list.
    ///
    /// # Safety
    /// - `slab_index` must be a valid index into the `list`.
    pub unsafe fn push(&mut self, slab_index: u32) {
        let current_head = *self.head;

        {
            // SAFETY: The `slab_index` is assumed to be a valid index into the `list`.
            let pushed_node = unsafe { self.get_unchecked(slab_index) };
            pushed_node
                .worker_local_prev
                .store(NULL_U32, Ordering::Release);
            pushed_node
                .worker_local_next
                .store(current_head, Ordering::Release);
        }

        if current_head != NULL_U32 {
            // SAFETY: The `current_head` is assumed to be a valid index into the `list`.
            let current_head_ref = unsafe { self.get_unchecked(current_head) };
            current_head_ref
                .worker_local_prev
                .store(slab_index, Ordering::Release);
        }

        *self.head = slab_index;
    }

    /// Remove a slab from the list.
    ///
    /// # Safety
    /// - `slab_index` must be a valid index into the `list`.
    /// - `slab_index` must be present in the list.
    pub unsafe fn remove(&mut self, slab_index: u32) {
        let (prev, next) = {
            // SAFETY: The `slab_index` is assumed to be a valid index into the `list`.
            let current_node = unsafe { self.get_unchecked(slab_index) };
            (
                current_node
                    .worker_local_prev
                    .swap(NULL_U32, Ordering::AcqRel),
                current_node
                    .worker_local_next
                    .swap(NULL_U32, Ordering::AcqRel),
            )
        };

        if prev != NULL_U32 {
            // SAFETY: The `prev` is assumed to be a valid index into the `list`.
            let prev_node = unsafe { self.get_unchecked(prev) };
            prev_node.worker_local_next.store(next, Ordering::Release);
        } else {
            // If there is no previous node, we are at the head.
            *self.head = next;
        };

        if next != NULL_U32 {
            // SAFETY: The `next` is assumed to be a valid index into the `list`.
            let next_node = unsafe { self.get_unchecked(next) };
            next_node.worker_local_prev.store(prev, Ordering::Release);
        }
    }

    /// Iterates over the worker local list.
    pub fn iterate(&self) -> impl Iterator<Item = u32> + '_ {
        let mut current_head = *self.head;
        core::iter::from_fn(move || {
            if current_head == NULL_U32 {
                return None; // End of the list
            }
            let ret = Some(current_head);
            // SAFETY: The `current_head` is assumed to be a valid index into the `list`.
            let current_node = unsafe { self.get_unchecked(current_head) };
            let next_index = current_node.worker_local_next.load(Ordering::Acquire);
            current_head = next_index;
            ret
        })
    }

    /// Get reference to a specific slab indexes `FreeListElement`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index into the `list`.
    unsafe fn get_unchecked(&self, slab_index: u32) -> &FreeListElement {
        self.list.add(slab_index as usize).as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicU32;

    #[test]
    fn test_worker_local_list() {
        const LIST_CAPACITY: u32 = 1024;
        let mut head = NULL_U32;
        let mut buffer = (0..LIST_CAPACITY)
            .map(|_| FreeListElement {
                global_next: AtomicU32::new(NULL_U32),
                worker_local_prev: AtomicU32::new(NULL_U32),
                worker_local_next: AtomicU32::new(NULL_U32),
            })
            .collect::<Vec<_>>();
        let mut worker_local_list =
            unsafe { WorkerLocalList::new(&mut head, NonNull::new(buffer.as_mut_ptr()).unwrap()) };

        // Test removal orders:
        // - [head, head, head]
        // - [tail, tail, tail]
        // - [middle, head, head]
        for removal_order in [[2, 1, 0], [0, 1, 2], [1, 2, 0], [1, 0, 2]] {
            unsafe {
                worker_local_list.push(0);
                assert_eq!(worker_local_list.iterate().collect::<Vec<_>>(), vec![0]);

                worker_local_list.push(1);
                assert_eq!(worker_local_list.iterate().collect::<Vec<_>>(), vec![1, 0]);

                worker_local_list.push(2);
                assert_eq!(
                    worker_local_list.iterate().collect::<Vec<_>>(),
                    vec![2, 1, 0]
                );

                let mut expected_order = vec![2, 1, 0];
                for index in removal_order {
                    worker_local_list.remove(index);
                    expected_order.retain(|&x| x != index);
                    assert_eq!(
                        worker_local_list.iterate().collect::<Vec<_>>(),
                        expected_order,
                        "mismatch for order: {removal_order:?}"
                    );
                }
            }
        }
    }
}

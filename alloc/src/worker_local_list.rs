use crate::index::NULL_U32;

/// A doubly linked-list that tracks slabs assigned to a worker.
/// This list is not safe to use concurrently across processes,
/// and should only be accessed by the associated worker.
#[repr(C)]
pub struct WorkerLocalList {
    head: u32,
    list: [LinkedListNode; 0],
}

#[repr(C)]
pub struct LinkedListNode {
    prev: u32,
    next: u32,
}

impl WorkerLocalList {
    /// Return the minimum byte size of the worker local list with
    /// given `capacity`.
    pub const fn byte_size(capacity: usize) -> usize {
        core::mem::size_of::<WorkerLocalList>() + capacity * core::mem::size_of::<LinkedListNode>()
    }

    /// Initializes the worker local list as empty with given `capacity`.
    /// # Safety
    /// - `capacity` must be a valid size for the `list`.
    pub unsafe fn initialize_as_empty(&mut self, capacity: u32) {
        self.head = NULL_U32;
        for slab_index in 0..capacity {
            // SAFETY: The `slab_index` is a valid index into the `list
            unsafe {
                let node = self.get_unchecked_mut(slab_index);
                node.prev = NULL_U32;
                node.next = NULL_U32;
            }
        }
    }

    /// Pushes `slab_index` onto the head of the worker local list.
    ///
    /// # Safety
    /// - `slab_index` must be a valid index into the `list`.
    pub unsafe fn push(&mut self, slab_index: u32) {
        let current_head = self.head;

        {
            let pushed_node = unsafe { self.get_unchecked_mut(slab_index) };
            pushed_node.prev = NULL_U32;
            pushed_node.next = current_head;
        }

        if current_head != NULL_U32 {
            // SAFETY: The `current_head` is assumed to be a valid index into the `list`.
            let current_head_ref = unsafe { self.get_unchecked_mut(current_head) };
            debug_assert_eq!(
                current_head_ref.prev, NULL_U32,
                "current head should have NULL prev"
            );
            current_head_ref.prev = slab_index;
        }

        self.head = slab_index;
    }

    /// Remove a slab from the list.
    ///
    /// # Safety
    /// - `slab_index` must be a valid index into the `list`.
    /// - `slab_index` must be present in the list.
    pub unsafe fn remove(&mut self, slab_index: u32) {
        let (prev, next) = {
            // SAFETY: The `slab_index` is assumed to be a valid index into the `list`.
            let current_node = unsafe { self.get_unchecked_mut(slab_index) };
            (
                core::mem::replace(&mut current_node.prev, NULL_U32),
                core::mem::replace(&mut current_node.next, NULL_U32),
            )
        };

        if prev != NULL_U32 {
            // SAFETY: The `prev` is assumed to be a valid index into the `list`.
            let prev_node = unsafe { self.get_unchecked_mut(prev) };
            prev_node.next = next;
        } else {
            // If there is no previous node, we are at the head.
            self.head = next;
        };

        if next != NULL_U32 {
            // SAFETY: The `next` is assumed to be a valid index into the `list`.
            let next_node = unsafe { self.get_unchecked_mut(next) };
            next_node.prev = prev;
        }
    }

    pub unsafe fn iterate(&self) -> impl Iterator<Item = u32> + '_ {
        let mut current_head = self.head;
        core::iter::from_fn(move || {
            if current_head == NULL_U32 {
                return None; // End of the list
            }
            let ret = Some(current_head);
            let current_node = unsafe { self.get_unchecked(current_head) };
            let next_index = current_node.next;
            current_head = next_index;
            ret
        })
    }

    /// Get reference to a specific slab indexes `LinkedListNode`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index into the `list`.
    unsafe fn get_unchecked(&self, slab_index: u32) -> &LinkedListNode {
        &*self.list.as_ptr().add(slab_index as usize)
    }

    /// Get mutable reference to a specific slab indexes `LinkedListNode`.
    ///
    /// # Safety
    /// - The `slab_index` must be a valid index into the `list`.
    unsafe fn get_unchecked_mut(&mut self, slab_index: u32) -> &mut LinkedListNode {
        &mut *self.list.as_mut_ptr().add(slab_index as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_local_list() {
        const LIST_CAPACITY: u32 = 1024;
        let mut buffer = [0u8; WorkerLocalList::byte_size(LIST_CAPACITY as usize)];
        let worker_local_list = unsafe { &mut *(buffer.as_mut_ptr() as *mut WorkerLocalList) };
        unsafe {
            worker_local_list.initialize_as_empty(LIST_CAPACITY);
        }

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

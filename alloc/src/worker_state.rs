use crate::{index::NULL_U32, size_classes::NUM_SIZE_CLASSES};

#[repr(C)]
pub struct WorkerState {
    pub partial_slabs_heads: [u32; NUM_SIZE_CLASSES],
    pub full_slabs_heads: [u32; NUM_SIZE_CLASSES],
}

impl WorkerState {
    /// Get the current head of the partial slabs for the given size index.
    /// Returns `None` if there are no partial slabs for that size index.
    ///
    /// # Safety
    /// - The `size_index` must be a valid index for the size classes.
    pub unsafe fn partial_head(&self, size_index: usize) -> Option<u32> {
        let current_head = *self.partial_slabs_heads.get_unchecked(size_index);
        if current_head == NULL_U32 {
            None
        } else {
            Some(current_head)
        }
    }
}

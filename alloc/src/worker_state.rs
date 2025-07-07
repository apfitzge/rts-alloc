use crate::{cache_aligned::CacheAlignedU32, size_classes::NUM_SIZE_CLASSES};

#[repr(C)]
pub struct WorkerState {
    pub partial_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
    pub full_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
}

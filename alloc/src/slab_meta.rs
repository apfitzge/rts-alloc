use crate::cache_aligned::CacheAlignedU32;
use core::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicUsize;

#[repr(C)]
pub struct SlabMeta {
    pub assigned_worker: AtomicU32,
    pub size_class_index: AtomicUsize,
    pub remote_free_stack_head: CacheAlignedU32,
}

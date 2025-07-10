use crate::cache_aligned::CacheAlignedU32;

pub const MAGIC: u64 = 0x727473616c6f63; // "rtsaloc"
pub const VERSION: u32 = 1;

#[repr(C)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub num_workers: u32,
    pub num_slabs: u32,
    pub global_free_stack_list_offset: u32,

    // pub slab_meta_offset: u32,
    // pub slab_meta_size: u32,
    // pub slab_size: u32,
    // pub slab_offset: u32,
    pub global_free_stack_head: CacheAlignedU32,
}

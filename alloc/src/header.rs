use crate::{cache_aligned::CacheAlignedU32, worker_state::WorkerState};

pub const MAGIC: u64 = 0x727473616c6f63; // "rtsaloc"
pub const VERSION: u32 = 1;

#[repr(C)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub num_workers: u32,
    pub num_slabs: u32,
    pub slab_meta_offset: u32,
    pub slab_meta_size: u32,
    pub slab_size: u32,
    pub slab_offset: u32,
    pub global_free_stack: CacheAlignedU32,

    /// Trailing array of worker states.
    /// Length is `num_workers`.
    pub worker_states: [WorkerState; 0],
    // trailing array of `SlabMeta` with size of `num_slabs`.
    // each is sufficiently sized such that we can hold a
    // `FreeStack` with size of 64 bytes + (slab_size / 256) u16s.
}

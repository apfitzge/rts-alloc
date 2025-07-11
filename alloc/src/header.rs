use crate::cache_aligned::CacheAlignedU32;

pub const MAGIC: u64 = 0x727473616c6f63; // "rtsaloc"
pub const VERSION: u32 = 1;

#[repr(C)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    /// Maximum number of workers that can use this allocator.
    pub num_workers: u32,
    /// Number of slabs in the allocator.
    pub num_slabs: u32,
    /// The size in bytes of each slab.
    pub slab_size: u32,

    /// The head of the global free list.
    pub global_free_list_head: CacheAlignedU32,
    /// The heads of the per-worker local free lists.
    pub worker_local_list_heads: [CacheAlignedU32; 0],
}

// Layout of the allocator.
// Padding used to ensure proper alignment between components.
//
// [header]
// [global_free_list_head]
// [worker_local_list_heads; num_workers]
// [global_free_list; num_slabs]
// [worker_local_list; num_slabs]
// [slab_shared_meta]
// [slab_free_stacks]
// [slabs]
mod layout {
    use crate::{align::round_to_next_alignment_of, global_free_list::GlobalFreeList};

    /// The size of the header in bytes.
    pub const fn header_size() -> usize {
        core::mem::size_of::<super::Header>()
    }
}

// Layout of the local lists for each worker.
//
// [worker_0_ll_for_size_class_0][worker_0_ll_for_size_class_1]...[worker_0_ll_for_size_class_n]
// [worker_1_ll_for_size_class_0][worker_1_ll_for_size_class_1]...[worker_1_ll_for_size_class_n]
// ...
//
// [worker_n_ll_for_size_class_0][worker_n_ll_for_size_class_1]...[worker_n_ll_for_size_class_n]
mod worker_local_lists {
    use crate::worker_local_list::WorkerLocalList;

    /// Calculate the total size in bytes of the worker local lists for all workers.
    pub const fn total_size(num_workers: u32, num_slabs: u32) -> usize {
        // WorkerLocalList::byte_size(num_slabs as usize) * num_workers as usize * num_slabs as usize
        todo!()
    }

    /// Calculate the offset of the worker local list for a given `worker_index` and `size_class_index`.
    pub const fn offset_of(worker_index: u32, size_class_index: usize) -> usize {
        todo!()
    }
}

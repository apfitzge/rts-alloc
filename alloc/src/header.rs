use crate::{
    cache_aligned::{CacheAligned, CacheAlignedU32},
    size_classes::NUM_SIZE_CLASSES,
};
use core::sync::atomic::AtomicU32;

pub const MAGIC: u64 = 0x727473616c6f63; // "rtsaloc"
pub const VERSION: u32 = 1;

pub type WorkerLocalListHeads = CacheAligned<[u32; NUM_SIZE_CLASSES]>;

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

    /// The offset in bytes to the free list elements.
    pub free_list_elements_offset: u32,
    /// The offset in bytes to the slab shared metadata.
    pub slab_shared_meta_offset: u32,
    /// The offset in bytes to the slab free stacks.
    pub slab_free_stacks_offset: u32,
    /// The offset in bytes to the slabs.
    pub slabs_offset: u32,

    /// The head of the global free list.
    pub global_free_list_head: CacheAlignedU32,
    /// The heads of the per-worker local free lists.
    pub worker_local_list_heads: [WorkerLocalListHeads; 0],
}

// Layout of the allocator.
// Padding used to ensure proper alignment between components.
//
// [header]
// [worker_local_list_heads; num_workers]
// [free_list_elements; num_slabs]
// [slab_shared_meta]
// [slab_free_stacks]
// [slabs]
pub mod layout {
    use crate::{
        align::round_to_next_alignment_of,
        free_list_element::FreeListElement,
        free_stack::FreeStack,
        header::{Header, WorkerLocalListHeads},
        slab_meta::SlabMeta,
    };

    /// The size of the header in bytes.
    pub const fn header_size() -> usize {
        core::mem::size_of::<Header>()
    }

    /// The size of the worker local list heads in bytes with trailing padding.
    pub const fn padded_worker_local_list_heads_size(num_workers: u32) -> usize {
        const FREE_LIST_ELEMENT_ALIGNMENT: usize = core::mem::align_of::<FreeListElement>();
        let size = core::mem::size_of::<WorkerLocalListHeads>() * num_workers as usize;
        round_to_next_alignment_of::<FREE_LIST_ELEMENT_ALIGNMENT>(size)
    }

    /// The size of the free list elements in bytes with trailing padding.
    pub const fn padded_free_list_elements_size(num_slabs: u32) -> usize {
        const SLAB_META_ALIGNMENT: usize = core::mem::align_of::<SlabMeta>();
        let size = core::mem::size_of::<FreeListElement>() * num_slabs as usize;
        round_to_next_alignment_of::<SLAB_META_ALIGNMENT>(size)
    }

    /// The size of the slab meta in bytes with trailing padding.
    pub const fn padded_slab_meta_size(num_slabs: u32) -> usize {
        const FREE_STACK_ALIGNMENT: usize = core::mem::align_of::<FreeStack>();
        let size = core::mem::size_of::<SlabMeta>() * num_slabs as usize;
        round_to_next_alignment_of::<FREE_STACK_ALIGNMENT>(size)
    }

    /// The size of the free stacks in bytes WITHOUT trailing padding.
    pub const fn free_stacks_size(num_slabs: u32) -> usize {
        FreeStack::byte_size(num_slabs as u16)
    }
}

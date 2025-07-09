use crate::{error::Error, raw_allocator::RawAllocator};
use core::ptr::NonNull;

pub struct Allocator {
    raw: RawAllocator,
    worker_index: u32,
}

impl Allocator {
    /// Creates a new `Allocator` for the given worker index.
    ///
    /// # Safety
    /// - The `raw` allocator must be initialized and valid.
    /// - The `worker_index` must be less than the number of workers in the `raw` allocator.
    /// - The `worker_index` must be uniquely assigned to this `Allocator` instance.
    pub unsafe fn new(raw: RawAllocator, worker_index: u32) -> Result<Self, Error> {
        if worker_index >= raw.header().num_workers {
            return Err(Error::InvalidWorkerIndex);
        }
        Ok(Allocator { raw, worker_index })
    }

    /// Allocates a block of memory of the given size.
    /// If the size is larger than the maximum size class, returns `None`.
    /// If the allocation fails, returns `None`.
    pub fn allocate(&self, size: u32) -> Option<NonNull<u8>> {
        None
    }
}

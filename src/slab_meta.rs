use crate::sync::{AtomicU32, AtomicUsize, Ordering};

#[repr(C)]
pub struct SlabMeta {
    pub assigned_worker: AtomicU32,
    pub size_class_index: AtomicUsize,
}

impl SlabMeta {
    pub fn assign(&self, worker_index: u32, size_class_index: usize) {
        self.assigned_worker.store(worker_index, Ordering::Release);
        self.size_class_index
            .store(size_class_index, Ordering::Release);
    }
}

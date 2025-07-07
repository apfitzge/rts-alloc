use crate::{
    free_stack::FreeStack, remote_free_stack::RemoteFreeStack, size_classes::SIZE_CLASSES,
};

#[repr(C)]
pub struct SlabMeta {
    pub prev: u32,            // used for intrusive linked-lists (worker_local)
    pub next: u32,            // used for intrusive linked-lists (global_free_stack, worker_local)
    pub assigned_worker: u32, // worker assigned to this slab (if any)
    pub size_class_index: u8, // index into SIZE_CLASSES
    pub remote_free_stack: RemoteFreeStack,
    pub free_stack: FreeStack,
}

impl SlabMeta {
    pub fn assign(&mut self, slab_size: u32, worker: u32, size_class_index: u8) {
        self.assigned_worker = worker;
        self.size_class_index = size_class_index;
        self.remote_free_stack.reset();
        unsafe {
            self.free_stack
                .reset(Self::capacity(slab_size, size_class_index));
        }
    }

    pub fn capacity(slab_size: u32, size_class_index: u8) -> u16 {
        let size_class = SIZE_CLASSES[size_class_index as usize];
        (slab_size / size_class) as u16
    }
}

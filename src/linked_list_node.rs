use core::sync::atomic::AtomicU32;

#[repr(C)]
pub struct LinkedListNode {
    pub global_next: AtomicU32,
    pub worker_local_prev: AtomicU32,
    pub worker_local_next: AtomicU32,
}

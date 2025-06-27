use std::{
    os::fd::AsRawFd,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{
    align::round_to_next_alignment_of,
    cache_aligned::{CacheAligned, CacheAlignedU32},
    free_stack::FreeStack,
    size_classes::{MIN_SIZE, NUM_SIZE_CLASSES, SIZE_CLASSES},
};

mod align;
pub mod cache_aligned;
pub mod free_stack;
pub mod size_classes;

pub struct Allocator {
    header: NonNull<Header>,
}

impl Allocator {
    pub fn header(&self) -> &Header {
        unsafe { self.header.as_ref() }
    }

    /// Take a free slab for the worker, for a specific size class.
    pub fn take_free_slab(&self, worker_index: usize, size_class_index: usize) -> bool {
        let Some(slab_index) = self.try_pop_free_slab() else {
            return false;
        };

        // No other worker should be touching this workers partial/full lists.
        // No need to do a CAS, since there should not be contention.
        let worker = unsafe { self.worker_state(worker_index).as_ref() };
        let partial_head = &worker.partial_slabs_heads[size_class_index as usize];
        let current_head = partial_head.load(Ordering::Relaxed);

        let slab_meta = unsafe { self.slab_meta(slab_index).as_mut() };
        slab_meta.next = current_head; // link the slab to the worker's partial list.
        unsafe {
            slab_meta
                .free_stack
                .reset((self.header().slab_size / SIZE_CLASSES[size_class_index]) as u16);
        };

        return true;
    }

    /// Push a slab index onto the global free stack.
    pub unsafe fn return_the_slab(&self, index: u32) {
        loop {
            let current_head = self.header().global_free_stack.load(Ordering::Acquire);
            unsafe { self.slab_meta(index).as_mut().next = current_head };
            if self
                .header()
                .global_free_stack
                .compare_exchange(current_head, index, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Try to pop a free slab index from the global free stack.
    /// Returns `None` if the stack is empty.
    pub fn try_pop_free_slab(&self) -> Option<u32> {
        let header = self.header();
        loop {
            let current_head = header.global_free_stack.load(Ordering::Acquire);
            if current_head == NULL {
                return None;
            }

            let next_slab = unsafe { self.slab_meta(current_head).as_ref().next };
            if header
                .global_free_stack
                .compare_exchange(current_head, next_slab, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(current_head);
            }
        }
    }

    /// Given a slab index, return a pointer to the slab metadata.
    pub unsafe fn slab_meta(&self, index: u32) -> NonNull<SlabMeta> {
        let header = self.header();
        self.header
            .cast::<u8>()
            .add(header.slab_meta_offset as usize)
            .add(index as usize * header.slab_meta_size as usize)
            .cast()
    }

    /// Given a slab index, return a pointer to the slab memory.
    pub unsafe fn slab(&self, index: u32) -> NonNull<u8> {
        let header = self.header();
        self.header
            .cast::<u8>()
            .add(header.slab_offset as usize)
            .add((index * header.slab_size) as usize)
    }

    /// Given a worker index, return a pointer to the worker state.
    pub unsafe fn worker_state(&self, index: usize) -> NonNull<WorkerState> {
        let header = self.header();
        let worker_states_ptr = header.worker_states.as_ptr();
        let worker_state_ptr = unsafe { worker_states_ptr.add(index) };
        NonNull::new(worker_state_ptr.cast_mut()).expect("Worker state pointer should not be null")
    }
}

pub fn create_allocator(
    file_path: impl AsRef<Path>,
    num_workers: u32,
    slab_size: u32,
    size: usize,
) -> Result<Allocator, ()> {
    // TODO: make error instead of panic.
    assert!(
        slab_size.is_power_of_two(),
        "Slab size must be a power of two"
    );

    let file_path = file_path.as_ref();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(file_path)
        .map_err(|_| ())?;
    file.set_len(size as u64).map_err(|_| ())?;

    let mmap = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };

    if mmap == libc::MAP_FAILED {
        return Err(());
    }

    let header_ptr = mmap as *mut Header;
    let mut header = NonNull::new(header_ptr).ok_or(())?;

    const WORKER_STATES_OFFSET: usize = core::mem::offset_of!(Header, worker_states);
    let worker_states_size = (num_workers as usize) * core::mem::size_of::<WorkerState>();
    const SLAB_META_ALIGNMENT: usize = core::mem::align_of::<SlabMeta>();
    let slab_meta_offset = (WORKER_STATES_OFFSET + worker_states_size + SLAB_META_ALIGNMENT - 1)
        & !(SLAB_META_ALIGNMENT - 1);

    // Each SlabMeta is aligned to SLAB_META_ALIGNMENT
    let slab_meta_size = core::mem::size_of::<SlabMeta>()
        + (slab_size / MIN_SIZE) as usize * core::mem::size_of::<u16>();
    // round to next multiple of alignment.
    let slab_meta_size: usize = round_to_next_alignment_of::<SLAB_META_ALIGNMENT>(slab_meta_size);

    // Total header and meta size - round to next multiple of slab size.
    let mask = (slab_size - 1) as usize;
    let total_meta_size = (slab_meta_offset + slab_meta_size + mask) & !mask;

    let slab_offset = total_meta_size as u32;
    let num_slabs = (size as u32 - slab_offset) / slab_size;

    unsafe {
        (*header.as_mut()).magic = MAGIC;
        (*header.as_mut()).version = VERSION;
        (*header.as_mut()).num_workers = num_workers;
        (*header.as_mut()).num_slabs = num_slabs;
        (*header.as_mut()).slab_meta_size = slab_meta_size as u32;
        (*header.as_mut()).slab_meta_offset = slab_meta_offset as u32;
        (*header.as_mut()).slab_size = slab_size;
        (*header.as_mut()).slab_offset = slab_offset;
        (*header.as_mut()).global_free_stack = CacheAligned(AtomicU32::new(NULL));
    }

    let allocator = Allocator { header };
    // Initialize slabs in the global free stack.
    for index in (0..num_slabs).rev() {
        unsafe { allocator.return_the_slab(index) };
    }

    // Initialize worker states.
    for worker_index in 0..num_workers {
        let mut worker_state = unsafe { allocator.worker_state(worker_index as usize) };
        for size_index in 0..NUM_SIZE_CLASSES {
            unsafe {
                worker_state.as_mut().partial_slabs_heads[size_index]
                    .store(NULL, Ordering::Relaxed);
                worker_state.as_mut().full_slabs_heads[size_index].store(NULL, Ordering::Relaxed);
            }
        }
    }

    // Slab metas do not need initialization since they are not used until a slab is allocated.

    Ok(Allocator { header })
}

pub fn join_allocator(file_path: impl AsRef<Path>) -> Result<Allocator, ()> {
    let file_path = file_path.as_ref();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(file_path)
        .map_err(|_| ())?;

    let size = file.metadata().map_err(|_| ())?.len() as usize;

    let mmap = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };

    if mmap == libc::MAP_FAILED {
        return Err(());
    }

    let header_ptr = mmap as *mut Header;
    let header = NonNull::new(header_ptr).ok_or(())?;
    Ok(Allocator { header })
}

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

#[repr(C)]
pub struct WorkerState {
    pub partial_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
    pub full_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
}

#[repr(C)]
pub struct SlabMeta {
    pub next: u32, // used for intrusive linked-lists
    pub free_stack: FreeStack,
}

const NULL: u32 = u32::MAX;

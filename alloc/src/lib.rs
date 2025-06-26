use std::{
    os::fd::AsRawFd,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

pub const NUM_SIZE_CLASSES: usize = 5;
/// Size classes must be sub powers of two
pub const SIZE_CLASSES: [u32; NUM_SIZE_CLASSES] = [256, 512, 1024, 2048, 4096];
const MAX_SIZE: u32 = SIZE_CLASSES[NUM_SIZE_CLASSES - 1];
const BASE_SHIFT: u32 = SIZE_CLASSES[0].trailing_zeros() as u32;

pub fn size_class_index(size: u32) -> u32 {
    debug_assert!(size <= MAX_SIZE);
    size.next_power_of_two()
        .trailing_zeros()
        .saturating_sub(BASE_SHIFT)
}

pub struct Allocator {
    header: NonNull<Header>,
}

impl Allocator {
    pub fn header(&self) -> &Header {
        unsafe { self.header.as_ref() }
    }

    /// Push a slab index onto the global free stack.
    pub unsafe fn return_the_slab(&self, index: u32) {
        let slab = self.slab(index).cast::<u32>();
        loop {
            let current_head = self.header().global_free_stack.load(Ordering::Acquire);
            slab.write(current_head);
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

            let next_slab = unsafe { self.slab(current_head).cast::<u32>().read() };
            if header
                .global_free_stack
                .compare_exchange(current_head, next_slab, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(current_head);
            }
        }
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

    // TODO: make error instead of panic.
    assert!(
        slab_size.is_power_of_two(),
        "Slab size must be a power of two"
    );
    let slab_offset = (core::mem::size_of::<Header>() as u32 + (slab_size - 1)) & !(slab_size - 1);
    let num_slabs = (size as u32 - slab_offset) / slab_size;

    unsafe {
        (*header.as_mut()).magic = MAGIC;
        (*header.as_mut()).version = VERSION;
        (*header.as_mut()).num_workers = num_workers;
        (*header.as_mut()).num_slabs = num_slabs;
        (*header.as_mut()).slab_size = slab_size;
        (*header.as_mut()).slab_offset = slab_offset;
        (*header.as_mut()).global_free_stack = CacheAlignedU32(AtomicU32::new(NULL));
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
    pub slab_size: u32,
    pub slab_offset: u32,
    pub global_free_stack: CacheAlignedU32,

    /// Trailing array of worker states.
    /// Length is `num_workers`.
    pub worker_states: [WorkerState; 0],
}

#[repr(C)]
pub struct WorkerState {
    pub partial_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
    pub full_slabs_heads: [CacheAlignedU32; NUM_SIZE_CLASSES],
}

#[repr(C, align(64))]
pub struct CacheAlignedU32(AtomicU32);

impl core::ops::Deref for CacheAlignedU32 {
    type Target = AtomicU32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

const NULL: u32 = u32::MAX;

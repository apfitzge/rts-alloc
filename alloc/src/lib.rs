use std::{
    os::fd::AsRawFd,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

pub struct Allocator {
    header: NonNull<Header>,
}

impl Allocator {
    pub fn header(&self) -> &Header {
        unsafe { self.header.as_ref() }
    }

    /// Push a slab index onto the global free stack.
    unsafe fn push_free_slab(&self, index: u32) {
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

    /// Given a slab index, return a pointer to the slab memory.
    pub unsafe fn slab(&self, index: u32) -> NonNull<u8> {
        let header = self.header();
        self.header
            .cast::<u8>()
            .add(header.slab_offset as usize)
            .add((index * header.slab_size) as usize)
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
    for index in (0..num_slabs).rev() {
        unsafe { allocator.push_free_slab(index) };
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

pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub num_workers: u32,
    pub num_slabs: u32,
    pub slab_size: u32,
    pub slab_offset: u32,
    pub global_free_stack: CacheAlignedU32,
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

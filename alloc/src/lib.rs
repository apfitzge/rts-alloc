use std::{os::fd::AsRawFd, path::Path, ptr::NonNull};

pub struct Allocator {
    header: NonNull<Header>,
}

impl Allocator {
    pub fn header(&self) -> &Header {
        unsafe { self.header.as_ref() }
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

    unsafe {
        (*header.as_mut()).magic = MAGIC;
        (*header.as_mut()).version = VERSION;
        (*header.as_mut()).num_workers = num_workers;
        (*header.as_mut()).slab_size = slab_size;
        (*header.as_mut()).slab_offset = std::mem::size_of::<Header>() as u32;
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
    pub slab_size: u32,
    pub slab_offset: u32,
}

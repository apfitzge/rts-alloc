#[derive(Debug)]
pub enum Error {
    InvalidSlabSize,
    InvalidWorkerIndex,
    InvalidFileSize,
    InvalidHeader,
    IoError(std::io::Error),
    MMapError(usize),
}

#[derive(Debug)]
pub enum Error {
    InvalidSlabSize,
    InvalidWorkerIndex,
    InvalidFileSize,
    IoError(std::io::Error),
    MMapError(usize),
}

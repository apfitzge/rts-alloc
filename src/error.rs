#[derive(Debug)]
pub enum Error {
    InvalidSlabSize,
    InvalidNumWorkers,
    InvalidWorkerIndex,
    InvalidFileSize,
    InvalidHeader,
    IoError(std::io::Error),
    MMapError(usize),
}

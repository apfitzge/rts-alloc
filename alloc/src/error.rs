#[derive(Debug)]
pub enum Error {
    InvalidSlabSize,
    InvalidWorkerIndex,
    IoError(std::io::Error),
    MMapError(usize),
}

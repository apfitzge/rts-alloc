#[derive(Debug)]
pub enum Error {
    InvalidSlabSize,
    IoError(std::io::Error),
    MMapError(usize),
}

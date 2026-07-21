/// The way to compress rotated log files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompressionMethod {
    /// Compress rotated log files through XZ with a level from 0 to 9.
    Xz(u32),
}

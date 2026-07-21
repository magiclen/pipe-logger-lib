/// The way to rotate log files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RotateMethod {
    /// Rotate log files by a file size threshold in bytes.
    FileSize(u64),
}

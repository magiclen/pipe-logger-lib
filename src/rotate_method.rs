#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The way to rotate log files.
pub enum RotateMethod {
    /// Rotate log files by a file size threshold in bytes.
    FileSize(u64),
}

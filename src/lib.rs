/*!
# Pipe Logger Lib

Stores, rotates, and compresses process logs.

`count` includes the active log file, so a count of 10 keeps up to 9 rotated files.

Call [`PipeLogger::flush`] to wait for queued compression work while keeping the logger open.

Call [`PipeLogger::finish`] to wait for queued compression work and report its final result.

## Example

```rust,no_run
use std::{error::Error, fs, path::Path};

use pipe_logger_lib::{CompressionMethod, PipeLoggerBuilder, RotateMethod, Tee};

fn main() -> Result<(), Box<dyn Error>> {
    let folder = Path::new("logs");

    fs::create_dir_all(folder)?;

    let mut builder = PipeLoggerBuilder::new(folder.join("application.log"));

    builder
        .set_tee(Some(Tee::Stdout))
        .set_rotate(Some(RotateMethod::FileSize(1024 * 1024)))
        .set_count(Some(10))
        .set_compression(Some(CompressionMethod::Xz(6)));

    let mut logger = builder.build()?;

    logger.write_line("The application started.")?;
    logger.finish()?;

    Ok(())
}
```
*/

mod build_error;
mod compression_method;
mod compression_worker;
mod rotate_method;
mod rotated_file;

use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub use build_error::BuildError;
pub use compression_method::CompressionMethod;
use compression_worker::CompressionWorker;
use path_absolutize::Absolutize;
pub use rotate_method::RotateMethod;
use rotated_file::{RotatedFile, create_rotated_file, enforce_retention, scan_rotated_files};

/// The output stream that receives a copy of every log write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tee {
    /// Write a copy to standard output.
    Stdout,
    /// Write a copy to standard error.
    Stderr,
}

/// A builder for [`PipeLogger`].
#[derive(Debug, Clone)]
pub struct PipeLoggerBuilder {
    rotate:      Option<RotateMethod>,
    count:       Option<usize>,
    log_path:    PathBuf,
    compression: Option<CompressionMethod>,
    tee:         Option<Tee>,
}

impl PipeLoggerBuilder {
    /// Creates a builder for the given log path.
    #[inline]
    pub fn new(log_path: impl AsRef<Path>) -> Self {
        Self {
            rotate:      None,
            count:       None,
            log_path:    log_path.as_ref().to_path_buf(),
            compression: None,
            tee:         None,
        }
    }

    /// Returns the rotation method.
    #[inline]
    pub const fn rotate(&self) -> Option<RotateMethod> {
        self.rotate
    }

    /// Returns the total number of log files to keep.
    #[inline]
    pub const fn count(&self) -> Option<usize> {
        self.count
    }

    /// Returns the log path.
    #[inline]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Returns the compression method.
    #[inline]
    pub const fn compression(&self) -> Option<CompressionMethod> {
        self.compression
    }

    /// Returns the tee output stream.
    #[inline]
    pub const fn tee(&self) -> Option<Tee> {
        self.tee
    }

    /// Sets the rotation method.
    #[inline]
    pub const fn set_rotate(&mut self, rotate: Option<RotateMethod>) -> &mut Self {
        self.rotate = rotate;
        self
    }

    /// Sets the total number of log files to keep.
    #[inline]
    pub const fn set_count(&mut self, count: Option<usize>) -> &mut Self {
        self.count = count;
        self
    }

    /// Sets the compression method.
    #[inline]
    pub const fn set_compression(&mut self, compression: Option<CompressionMethod>) -> &mut Self {
        self.compression = compression;
        self
    }

    /// Sets the tee output stream.
    #[inline]
    pub const fn set_tee(&mut self, tee: Option<Tee>) -> &mut Self {
        self.tee = tee;
        self
    }

    /// Builds a logger.
    pub fn build(self) -> Result<PipeLogger, BuildError> {
        // validate
        {
            if matches!(self.rotate, Some(RotateMethod::FileSize(0))) {
                return Err(BuildError::RotateFileSizeZero);
            }

            if self.count == Some(0) {
                return Err(BuildError::CountZero);
            }

            if self.count.is_some() && self.rotate.is_none() {
                return Err(BuildError::CountWithoutRotation);
            }

            if self.compression.is_some() && self.rotate.is_none() {
                return Err(BuildError::CompressionWithoutRotation);
            }

            if let Some(CompressionMethod::Xz(level)) = self.compression
                && level > 9
            {
                return Err(BuildError::InvalidXzCompressionLevel(level));
            }
        }

        let file_path = self.log_path.absolutize()?.into_owned();

        match fs::metadata(&file_path) {
            Ok(metadata) if metadata.is_dir() => {
                return Err(BuildError::LogPathIsDirectory(file_path));
            },
            Ok(_) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }

        let folder_path = file_path
            .parent()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "The log path has no parent folder.")
            })?
            .to_path_buf();

        let file_name = file_path
            .file_name()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "The log path has no file name.")
            })?
            .to_os_string();

        let lock_path = lock_path(&folder_path, &file_name);

        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;

        match lock_file.try_lock() {
            Ok(()) => {},
            Err(fs::TryLockError::WouldBlock) => {
                return Err(BuildError::LogPathAlreadyInUse(file_path));
            },
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }

        let file = OpenOptions::new().create(true).append(true).open(&file_path)?;
        let file_size = file.metadata()?.len();

        let mut rotated_files = if self.rotate.is_some() {
            scan_rotated_files(&folder_path, &file_name)?
        } else {
            VecDeque::new()
        };

        let max_rotated_files = self.count.map_or(usize::MAX, |count| count - 1);

        let compression_worker = match self.compression {
            Some(CompressionMethod::Xz(level)) => Some(CompressionWorker::start(
                std::mem::take(&mut rotated_files),
                max_rotated_files,
                level,
            )?),
            None => {
                enforce_retention(&mut rotated_files, max_rotated_files)?;
                None
            },
        };

        Ok(PipeLogger {
            rotate: self.rotate,
            max_rotated_files,
            file: Some(file),
            _lock_file: lock_file,
            file_path,
            file_size,
            folder_path,
            file_name,
            rotated_files,
            compression_worker,
            tee: self.tee,
            finished: false,
        })
    }
}

/// A file logger with optional rotation, retention, compression, and tee output.
pub struct PipeLogger {
    rotate:             Option<RotateMethod>,
    max_rotated_files:  usize,
    file:               Option<File>,
    _lock_file:         File,
    file_path:          PathBuf,
    file_size:          u64,
    folder_path:        PathBuf,
    file_name:          OsString,
    rotated_files:      VecDeque<RotatedFile>,
    compression_worker: Option<CompressionWorker>,
    tee:                Option<Tee>,
    finished:           bool,
}

impl PipeLogger {
    /// Creates a builder for the given log path.
    #[inline]
    pub fn builder(log_path: impl AsRef<Path>) -> PipeLoggerBuilder {
        PipeLoggerBuilder::new(log_path)
    }

    /// Writes a string and rotates the log file when needed.
    #[inline]
    pub fn write_str(&mut self, text: &str) -> io::Result<Option<PathBuf>> {
        self.write_bytes(text.as_bytes(), false)
    }

    /// Writes a string and a newline, then rotates the log file when needed.
    #[inline]
    pub fn write_line(&mut self, text: &str) -> io::Result<Option<PathBuf>> {
        self.write_bytes(text.as_bytes(), true)
    }

    /// Flushes all output and waits for background work to finish.
    #[inline]
    pub fn flush(&mut self) -> io::Result<()> {
        self.flush_inner()
    }

    /// Flushes all output and stops the background worker.
    #[inline]
    pub fn finish(mut self) -> io::Result<()> {
        self.finish_inner()
    }

    fn write_bytes(&mut self, bytes: &[u8], append_newline: bool) -> io::Result<Option<PathBuf>> {
        if bytes.is_empty() && !append_newline {
            return Ok(None);
        }

        // Write to the file first so a broken tee pipe cannot drop a log line.
        let write_result = (|| {
            let file = self.file_mut()?;

            file.write_all(bytes)?;

            if append_newline {
                file.write_all(b"\n")?;
            }

            Ok(())
        })();

        if let Err(error) = write_result {
            self.refresh_file_size();

            return Err(error);
        }

        self.file_size = self.file_size.saturating_add(bytes.len() as u64);

        if append_newline {
            self.file_size = self.file_size.saturating_add(1);
        }

        write_tee(self.tee, bytes, append_newline)?;

        if self.should_rotate() { self.rotate() } else { Ok(None) }
    }

    #[inline]
    const fn should_rotate(&self) -> bool {
        match self.rotate {
            Some(RotateMethod::FileSize(file_size)) => self.file_size >= file_size,
            None => false,
        }
    }

    fn rotate(&mut self) -> io::Result<Option<PathBuf>> {
        let rotated_file = create_rotated_file(&self.folder_path, &self.file_name)?;
        let mut file = self.file.take().ok_or_else(file_unavailable_error)?;

        if let Err(error) = file.flush() {
            self.file = Some(file);

            return Err(error);
        }

        let permissions = match file.metadata() {
            Ok(metadata) => metadata.permissions(),
            Err(error) => {
                self.file = Some(file);

                return Err(error);
            },
        };

        drop(file);

        if let Err(error) = fs::rename(&self.file_path, &rotated_file.raw_path) {
            self.reopen_active_file();

            return Err(error);
        }

        let open_result = OpenOptions::new().append(true).create_new(true).open(&self.file_path);

        let new_file = match open_result {
            Ok(file) => file,
            Err(error) => {
                self.recover_from_new_file_error(&rotated_file, error.kind());

                return Err(error);
            },
        };

        let permission_result = new_file.set_permissions(permissions);

        self.file_size = 0;
        self.file = Some(new_file);

        let retention_result = self.accept_rotated_file(rotated_file.clone());

        permission_result?;
        retention_result?;

        if self.max_rotated_files == 0 {
            Ok(None)
        } else if self.compression_worker.is_some() {
            Ok(Some(rotated_file.compressed_path))
        } else {
            Ok(Some(rotated_file.raw_path))
        }
    }

    fn accept_rotated_file(&mut self, rotated_file: RotatedFile) -> io::Result<()> {
        match &self.compression_worker {
            Some(worker) => worker.rotate(rotated_file),
            None => {
                self.rotated_files.push_back(rotated_file);

                enforce_retention(&mut self.rotated_files, self.max_rotated_files)
            },
        }
    }

    fn recover_from_new_file_error(
        &mut self,
        rotated_file: &RotatedFile,
        error_kind: io::ErrorKind,
    ) {
        let rolled_back = error_kind != io::ErrorKind::AlreadyExists
            && fs::rename(&rotated_file.raw_path, &self.file_path).is_ok();

        self.reopen_active_file();

        if !rolled_back {
            let _ = self.accept_rotated_file(rotated_file.clone());
        }
    }

    fn reopen_active_file(&mut self) {
        self.file = OpenOptions::new().create(true).append(true).open(&self.file_path).ok();
        self.refresh_file_size();
    }

    fn refresh_file_size(&mut self) {
        self.file_size =
            self.file.as_ref().and_then(|file| file.metadata().ok()).map_or(0, |m| m.len());
    }

    #[inline]
    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file.as_mut().ok_or_else(file_unavailable_error)
    }

    fn flush_inner(&mut self) -> io::Result<()> {
        let output_result = self.flush_outputs();

        let background_result = if let Some(worker) = &self.compression_worker {
            worker.barrier()
        } else {
            enforce_retention(&mut self.rotated_files, self.max_rotated_files)
        };

        output_result.and(background_result)
    }

    fn finish_inner(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }

        let output_result = self.flush_outputs();

        let retention_result = if self.compression_worker.is_none() {
            enforce_retention(&mut self.rotated_files, self.max_rotated_files)
        } else {
            Ok(())
        };

        let worker_result = match &mut self.compression_worker {
            Some(worker) => worker.finish(),
            None => Ok(()),
        };

        self.finished = true;

        output_result.and(retention_result).and(worker_result)
    }

    fn flush_outputs(&mut self) -> io::Result<()> {
        let file_result = self.file_mut().and_then(Write::flush);
        let tee_result = flush_tee(self.tee);

        file_result.and(tee_result)
    }
}

impl Write for PipeLogger {
    #[inline]
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.write_bytes(buffer, false)?;

        Ok(buffer.len())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.flush_inner()
    }
}

impl Drop for PipeLogger {
    fn drop(&mut self) {
        let _ = self.finish_inner();
    }
}

fn lock_path(folder_path: &Path, file_name: &OsStr) -> PathBuf {
    let mut lock_file_name = file_name.to_os_string();

    lock_file_name.push(".pipe-logger.lock");
    folder_path.join(lock_file_name)
}

fn write_tee(tee: Option<Tee>, bytes: &[u8], append_newline: bool) -> io::Result<()> {
    let Some(tee) = tee else {
        return Ok(());
    };

    match tee {
        Tee::Stdout => write_output(io::stdout().lock(), bytes, append_newline),
        Tee::Stderr => write_output(io::stderr().lock(), bytes, append_newline),
    }
}

fn write_output(mut output: impl Write, bytes: &[u8], append_newline: bool) -> io::Result<()> {
    output.write_all(bytes)?;

    if append_newline {
        output.write_all(b"\n")?;
    }

    Ok(())
}

fn flush_tee(tee: Option<Tee>) -> io::Result<()> {
    match tee {
        Some(Tee::Stdout) => io::stdout().lock().flush(),
        Some(Tee::Stderr) => io::stderr().lock().flush(),
        None => Ok(()),
    }
}

fn file_unavailable_error() -> io::Error {
    io::Error::other("The active log file is unavailable after a failed rotation.")
}

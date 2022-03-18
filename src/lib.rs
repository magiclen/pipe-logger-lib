/*!
# Pipe Logger Lib
Stores, rotates, compresses process logs.

## Example

```rust
use pipe_logger_lib::*;

use std::fs;
use std::path::Path;

let test_folder = {
  let folder = Path::join(&Path::join(Path::new("tests"), Path::new("out")), "log-example");

  fs::remove_dir_all(&folder);

  fs::create_dir_all(&folder).unwrap();

  folder
};

let test_log_file = Path::join(&test_folder, Path::new("mylog.txt"));

let mut builder = PipeLoggerBuilder::new(&test_log_file);

builder
    .set_tee(Some(Tee::Stdout))
    .set_rotate(Some(RotateMethod::FileSize(30))) // bytes
    .set_count(Some(10))
    .set_compress(false);

{
    let mut logger = builder.build().unwrap();

    logger.write_line("Hello world!").unwrap();

    let rotated_log_file_1 = logger.write_line("This is a convenient logger.").unwrap().unwrap();

    logger.write_line("Other logs...").unwrap();
    logger.write_line("Other logs...").unwrap();

    let rotated_log_file_2 = logger.write_line("Rotate again!").unwrap().unwrap();

    logger.write_line("Ops!").unwrap();
}

fs::remove_dir_all(test_folder).unwrap();
```

Now, the contents of `test_log_file` are,

```text
Ops!
```

The contents of `rotated_log_file_1` are,

```text
Hello world!
This is a convenient logger.
```

The contents of `rotated_log_file_2` are,

```text
Other logs...
Other logs...
Rotate again!
```
*/

mod rotate_method;

pub use rotate_method::RotateMethod;

use std::error::Error;
use std::fmt::{Display, Error as FmtError, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::prelude::*;
use path_absolutize::*;

use regex::Regex;

use xz2::write::XzEncoder;

const BUFFER_SIZE: usize = 4096 * 4;
const FILE_WAIT_MILLI_SECONDS: u64 = 30;

// TODO -----PipeLoggerBuilder START-----

#[derive(Debug)]
pub enum PipeLoggerBuilderError {
    /// A valid rotated file size needs bigger than 1.
    RotateFileSizeTooSmall,
    /// A valid count of log files needs bigger than 0.
    CountTooSmall,
    /// std::io::Error.
    IOError(io::Error),
    /// A log file cannot be a directory. Wrap the absolutized log file.
    FileIsDirectory(PathBuf),
}

impl Display for PipeLoggerBuilderError {
    #[inline]
    fn fmt(&self, f: &mut Formatter) -> Result<(), FmtError> {
        match self {
            PipeLoggerBuilderError::RotateFileSizeTooSmall => {
                f.write_str("A valid rotated file size needs bigger than 1.")
            }
            PipeLoggerBuilderError::CountTooSmall => {
                f.write_str("A valid count of log files needs bigger than 0.")
            }
            PipeLoggerBuilderError::IOError(err) => Display::fmt(err, f),
            PipeLoggerBuilderError::FileIsDirectory(path) => {
                f.write_fmt(format_args!(
                    "A log file cannot be a directory. The path of that file is `{}`.",
                    path.to_string_lossy()
                ))
            }
        }
    }
}

impl Error for PipeLoggerBuilderError {}

impl From<io::Error> for PipeLoggerBuilderError {
    #[inline]
    fn from(err: io::Error) -> Self {
        PipeLoggerBuilderError::IOError(err)
    }
}

impl From<PathBuf> for PipeLoggerBuilderError {
    #[inline]
    fn from(err: PathBuf) -> Self {
        PipeLoggerBuilderError::FileIsDirectory(err)
    }
}

#[derive(Debug, Clone)]
/// Read from standard input and write to standard output.
pub enum Tee {
    /// To stdout.
    Stdout,
    /// To stderr.
    Stderr,
}

#[derive(Debug)]
/// To build a PipeLogger instance.
pub struct PipeLoggerBuilder<P: AsRef<Path>> {
    rotate: Option<RotateMethod>,
    count: Option<usize>,
    log_path: P,
    compress: bool,
    tee: Option<Tee>,
}

impl<P: AsRef<Path>> PipeLoggerBuilder<P> {
    /// Create a new PipeLoggerBuilder.
    pub fn new(log_path: P) -> PipeLoggerBuilder<P> {
        PipeLoggerBuilder {
            rotate: None,
            count: None,
            log_path,
            compress: false,
            tee: None,
        }
    }

    pub fn rotate(&self) -> &Option<RotateMethod> {
        &self.rotate
    }

    pub fn count(&self) -> &Option<usize> {
        &self.count
    }

    pub fn log_path(&self) -> &P {
        &self.log_path
    }

    /// Whether to compress the rotated log files through xz.
    pub fn compress(&self) -> bool {
        self.compress
    }

    pub fn tee(&self) -> &Option<Tee> {
        &self.tee
    }

    pub fn set_rotate(&mut self, rotate: Option<RotateMethod>) -> &mut Self {
        self.rotate = rotate;
        self
    }

    pub fn set_count(&mut self, count: Option<usize>) -> &mut Self {
        self.count = count;
        self
    }

    /// Whether to compress the rotated log files through xz.
    pub fn set_compress(&mut self, compress: bool) -> &mut Self {
        self.compress = compress;
        self
    }

    pub fn set_tee(&mut self, tee: Option<Tee>) -> &mut Self {
        self.tee = tee;
        self
    }

    /// Build a new PipeLogger.
    pub fn build(self) -> Result<PipeLogger, PipeLoggerBuilderError> {
        if let Some(rotate) = &self.rotate {
            match rotate {
                RotateMethod::FileSize(file_size) => {
                    if *file_size < 2 {
                        return Err(PipeLoggerBuilderError::RotateFileSizeTooSmall);
                    }
                }
            }

            if let Some(count) = &self.count {
                if *count < 1 {
                    return Err(PipeLoggerBuilderError::CountTooSmall);
                }
            }
        }

        let file_path = self.log_path.as_ref().absolutize()?;

        let file_size;

        let folder_path = match file_path.metadata() {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return Err(PipeLoggerBuilderError::FileIsDirectory(file_path.into_owned()));
                }

                let p = metadata.permissions();

                if p.readonly() {
                    return Err(PipeLoggerBuilderError::IOError(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("`{}` is readonly.", file_path.to_str().unwrap()),
                    )));
                }

                file_size = metadata.len();

                match file_path.parent() {
                    Some(parent) => {
                        if self.rotate.is_some() {
                            match fs::metadata(&parent) {
                                Ok(m) => {
                                    let p = m.permissions();
                                    if p.readonly() {
                                        return Err(PipeLoggerBuilderError::IOError(
                                            io::Error::new(
                                                io::ErrorKind::PermissionDenied,
                                                format!(
                                                    "`{}` is readonly.",
                                                    parent.to_str().unwrap()
                                                ),
                                            ),
                                        ));
                                    }
                                }
                                Err(err) => {
                                    return Err(PipeLoggerBuilderError::IOError(err));
                                }
                            }
                        }
                        parent
                    }
                    None => unreachable!(),
                }
            }
            Err(_) => {
                file_size = 0;

                match file_path.parent() {
                    Some(parent) => {
                        match fs::metadata(&parent) {
                            Ok(m) => {
                                let p = m.permissions();
                                if p.readonly() {
                                    return Err(PipeLoggerBuilderError::IOError(io::Error::new(
                                        io::ErrorKind::PermissionDenied,
                                        format!("`{}` is readonly.", parent.to_str().unwrap()),
                                    )));
                                }
                                parent
                            }
                            Err(err) => {
                                return Err(PipeLoggerBuilderError::IOError(err));
                            }
                        }
                    }
                    None => {
                        return Err(PipeLoggerBuilderError::IOError(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("`{}`'s parent does not exist.", file_path.to_str().unwrap()),
                        )));
                    }
                }
            }
        }
        .to_path_buf();

        let file_name =
            Path::new(file_path.as_ref()).file_name().unwrap().to_str().unwrap().to_string();

        let file_name_point_index = match file_name.rfind('.') {
            Some(index) => index,
            None => file_name.len(),
        };

        let rotated_log_file_names = {
            let mut rotated_log_file_names = Vec::new();

            let re = Regex::new("^-[1-2][0-9]{3}(-[0-5][0-9]){5}-[0-9]{6}$").unwrap(); // -%Y-%m-%d-%H-%M-%S + $.3f

            let file_name_without_extension = &file_name[..file_name_point_index];

            for entry in folder_path.read_dir().unwrap().filter_map(|entry| entry.ok()) {
                let rotated_log_file_path = entry.path();

                if !rotated_log_file_path.is_file() {
                    continue;
                }

                let rotated_log_file_name =
                    Path::new(&rotated_log_file_path).file_name().unwrap().to_str().unwrap();

                if !rotated_log_file_name.starts_with(file_name_without_extension) {
                    continue;
                }

                let rotated_log_file_name_point_index = match rotated_log_file_name.rfind('.') {
                    Some(index) => index,
                    None => rotated_log_file_name.len(),
                };

                if rotated_log_file_name_point_index < file_name_point_index + 24 {
                    // -%Y-%m-%d-%H-%M-%S + $.3f
                    continue;
                }

                let file_name_without_extension_len = file_name_without_extension.len();

                if !re.is_match(
                    &rotated_log_file_name
                        [file_name_without_extension_len..file_name_without_extension_len + 24],
                ) {
                    // -%Y-%m-%d-%H-%M-%S + $.3f
                    continue;
                }

                let ext = &rotated_log_file_name[rotated_log_file_name_point_index..];

                if ext.eq(&file_name[file_name_point_index..]) {
                    rotated_log_file_names.push(rotated_log_file_name.to_string());
                } else if ext.eq(".xz")
                    && rotated_log_file_name[..rotated_log_file_name_point_index]
                        .ends_with(&file_name[file_name_point_index..])
                {
                    rotated_log_file_names.push(
                        rotated_log_file_name[..rotated_log_file_name_point_index].to_string(),
                    );
                }
            }

            rotated_log_file_names.sort_unstable();

            rotated_log_file_names
        };

        let file =
            OpenOptions::new().create(true).write(true).append(true).open(file_path.as_ref())?;

        Ok(PipeLogger {
            rotate: self.rotate,
            count: self.count,
            file: Some(file),
            file_name,
            file_name_point_index,
            file_path: file_path.into_owned(),
            file_size,
            folder_path,
            rotated_log_file_names,
            compress: self.compress,
            tee: self.tee,
            last_rotated_time: 0,
        })
    }
}

// TODO -----PipeLoggerBuilder END-----

// TODO -----PipeLogger START-----

/// PipeLogger can help you stores, rotates and compresses logs.
pub struct PipeLogger {
    rotate: Option<RotateMethod>,
    count: Option<usize>,
    file: Option<File>,
    file_name: String,
    file_name_point_index: usize,
    file_path: PathBuf,
    file_size: u64,
    folder_path: PathBuf,
    rotated_log_file_names: Vec<String>,
    compress: bool,
    tee: Option<Tee>,
    last_rotated_time: i64,
}

impl Write for PipeLogger {
    /// Write UTF-8 data.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        PipeLogger::write(self, String::from_utf8_lossy(buf))?;

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file {
            Some(ref mut file) => file.flush(),
            None => unreachable!(),
        }
    }
}

impl PipeLogger {
    /// Create a new PipeLoggerBuilder.
    pub fn builder<P: AsRef<Path>>(log_path: P) -> PipeLoggerBuilder<P> {
        PipeLoggerBuilder::new(log_path)
    }

    /// Write a string. If the log is rotated, this method returns the renamed path.
    pub fn write<S: AsRef<str>>(&mut self, text: S) -> io::Result<Option<PathBuf>> {
        let s = text.as_ref();

        let buf = s.as_bytes();

        let len = buf.len();

        if len == 0 {
            return Ok(None);
        }

        self.print(s);

        let mut file = self.file.take().unwrap();

        let n = file.write(buf)?;

        self.file_size += n as u64;

        let mut new_file = None;

        if let Some(rotate) = &self.rotate {
            match rotate {
                RotateMethod::FileSize(size) => {
                    if self.file_size >= *size {
                        let utc: DateTime<Utc> = {
                            let mut utc: DateTime<Utc> = Utc::now();
                            let mut millisecond = utc.timestamp_millis();
                            while self.last_rotated_time == millisecond {
                                // Especially for Windows, because its time precision is about 15ms.
                                thread::sleep(Duration::from_millis(FILE_WAIT_MILLI_SECONDS));
                                utc = Utc::now();
                                millisecond = utc.timestamp_millis();
                            }
                            self.last_rotated_time = millisecond;
                            utc
                        };

                        let timestamp = utc.format("%Y-%m-%d-%H-%M-%S").to_string();
                        let millisecond = utc.format("%.3f").to_string();

                        file.flush()?;

                        file.sync_all()?;

                        drop(file);

                        let rotated_log_file_name = format!(
                            "{}-{}-{}{}",
                            &self.file_name[..self.file_name_point_index],
                            timestamp,
                            &millisecond[1..],
                            &self.file_name[self.file_name_point_index..]
                        );

                        let rotated_log_file =
                            Path::join(&self.folder_path, Path::new(&rotated_log_file_name));

                        fs::copy(&self.file_path, &rotated_log_file)?;

                        if self.compress {
                            let rotated_log_file_name_compressed =
                                format!("{}.xz", rotated_log_file_name);
                            let rotated_log_file_compressed = Path::join(
                                &self.folder_path,
                                Path::new(&rotated_log_file_name_compressed),
                            );
                            let rotated_log_file = rotated_log_file.clone();

                            let tee = self.tee.clone();

                            let print_err = move |s| {
                                match tee {
                                    Some(tee) => {
                                        match tee {
                                            Tee::Stdout => {
                                                eprintln!("{}", s);
                                            }
                                            Tee::Stderr => {
                                                println!("{}", s);
                                            }
                                        }
                                    }
                                    None => {
                                        eprintln!("{}", s);
                                    }
                                }
                            };

                            thread::spawn(move || {
                                match File::create(&rotated_log_file_compressed) {
                                    Ok(file_w) => {
                                        match File::open(&rotated_log_file) {
                                            Ok(mut file_r) => {
                                                let mut compressor = XzEncoder::new(file_w, 9);
                                                let mut buffer = [0u8; BUFFER_SIZE];
                                                loop {
                                                    match file_r.read(&mut buffer) {
                                                        Ok(c) => {
                                                            if c == 0 {
                                                                drop(file_r);
                                                                if fs::remove_file(
                                                                    &rotated_log_file,
                                                                )
                                                                .is_err()
                                                                {
                                                                }
                                                                break;
                                                            }
                                                            match compressor.write(&buffer[..c]) {
                                                                Ok(cc) => {
                                                                    if c != cc {
                                                                        print_err("The space is not enough.".to_string());
                                                                        break;
                                                                    }
                                                                }
                                                                Err(err) => {
                                                                    print_err(err.to_string());
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                        Err(ref err)
                                                            if err.kind()
                                                                == io::ErrorKind::NotFound =>
                                                        {
                                                            // The rotated log file is deleted because of the count limit
                                                            drop(compressor);
                                                            if fs::remove_file(
                                                                &rotated_log_file_compressed,
                                                            )
                                                            .is_err()
                                                            {
                                                            }
                                                            break;
                                                        }
                                                        Err(err) => {
                                                            print_err(err.to_string());
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            Err(ref err)
                                                if err.kind() == io::ErrorKind::NotFound =>
                                            {
                                                // The rotated log file is deleted because of the count limit
                                                drop(file_w);
                                                if fs::remove_file(&rotated_log_file_compressed)
                                                    .is_err()
                                                {
                                                }
                                            }
                                            Err(err) => {
                                                print_err(err.to_string());
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        print_err(err.to_string());
                                    }
                                };
                            });
                        }

                        self.rotated_log_file_names.push(rotated_log_file_name);

                        if let Some(count) = self.count {
                            while self.rotated_log_file_names.len() >= count {
                                let mut rotated_log_file_name =
                                    self.rotated_log_file_names.remove(0);
                                if fs::remove_file(Path::join(
                                    &self.folder_path,
                                    Path::new(&rotated_log_file_name),
                                ))
                                .is_err()
                                {}

                                let p_compressed_name = {
                                    rotated_log_file_name.push_str(".xz");

                                    rotated_log_file_name
                                };

                                let p_compressed =
                                    Path::join(&self.folder_path, Path::new(&p_compressed_name));
                                if fs::remove_file(&p_compressed).is_err() {}
                            }
                        }

                        file =
                            OpenOptions::new().write(true).truncate(true).open(&self.file_path)?;

                        self.file_size = 0;

                        new_file = if self.compress {
                            let mut s = rotated_log_file.into_os_string();
                            s.push(".xz");
                            Some(PathBuf::from(s))
                        } else {
                            Some(rotated_log_file)
                        };
                    }
                }
            }
        }

        if n != len {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "The space is not enough."));
        }

        self.file = Some(file);

        Ok(new_file)
    }

    /// Write a string with a new line. If the log is rotated, this method returns the renamed path.
    pub fn write_line<S: AsRef<str>>(&mut self, text: S) -> io::Result<Option<PathBuf>> {
        let new_file = self.write(text)?;

        if new_file.is_none() {
            match self.file {
                Some(ref mut file) => {
                    let n = file.write(b"\n")?;

                    if n != 1 {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "The space is not enough.",
                        ));
                    }

                    self.file_size += 1u64;
                }
                None => unreachable!(),
            }
            self.print("\n");
        }

        Ok(new_file)
    }

    fn print<S: AsRef<str>>(&self, text: S) {
        let s = text.as_ref();

        match &self.tee {
            Some(tee) => {
                match tee {
                    Tee::Stdout => {
                        print!("{}", s);
                    }
                    Tee::Stderr => {
                        eprint!("{}", s);
                    }
                }
            }
            None => (),
        }
    }
}

// TODO -----PipeLogger END-----

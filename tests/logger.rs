use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicUsize, Ordering},
};

use liblzma::read::XzDecoder;
use pipe_logger_lib::{BuildError, CompressionMethod, PipeLoggerBuilder, RotateMethod, Tee};

const LOG_FILE_NAME: &str = "logfile.log";

static NEXT_TEST_FOLDER: AtomicUsize = AtomicUsize::new(0);

fn create_test_folder() -> PathBuf {
    let number = NEXT_TEST_FOLDER.fetch_add(1, Ordering::Relaxed);
    let folder = Path::new("tests").join("out").join(format!("{}-{number}", process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn log_path(folder: &Path) -> PathBuf {
    folder.join(LOG_FILE_NAME)
}

fn build_error(builder: PipeLoggerBuilder) -> BuildError {
    match builder.build() {
        Ok(_) => panic!("The builder unexpectedly succeeded."),
        Err(error) => error,
    }
}

fn read_xz(path: &Path) -> Vec<u8> {
    let mut decoder = XzDecoder::new(File::open(path).unwrap());
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn builder_api_and_validation() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);

    assert_eq!(None, builder.rotate());
    assert_eq!(None, builder.count());
    assert_eq!(path.as_path(), builder.log_path());
    assert_eq!(None, builder.compression());
    assert_eq!(None, builder.tee());

    builder
        .set_rotate(Some(RotateMethod::FileSize(128)))
        .set_count(Some(4))
        .set_compression(Some(CompressionMethod::Xz(6)))
        .set_tee(Some(Tee::Stderr));

    assert_eq!(Some(RotateMethod::FileSize(128)), builder.rotate());
    assert_eq!(Some(4), builder.count());
    assert_eq!(Some(CompressionMethod::Xz(6)), builder.compression());
    assert_eq!(Some(Tee::Stderr), builder.tee());

    let mut invalid = PipeLoggerBuilder::new(&path);
    invalid.set_rotate(Some(RotateMethod::FileSize(0)));
    assert!(matches!(build_error(invalid), BuildError::RotateFileSizeZero));

    let mut invalid = PipeLoggerBuilder::new(&path);
    invalid.set_count(Some(0));
    assert!(matches!(build_error(invalid), BuildError::CountZero));

    let mut invalid = PipeLoggerBuilder::new(&path);
    invalid.set_count(Some(2));
    assert!(matches!(build_error(invalid), BuildError::CountWithoutRotation));

    let mut invalid = PipeLoggerBuilder::new(&path);
    invalid.set_compression(Some(CompressionMethod::Xz(6)));
    assert!(matches!(build_error(invalid), BuildError::CompressionWithoutRotation));

    let mut invalid = PipeLoggerBuilder::new(&path);
    invalid
        .set_rotate(Some(RotateMethod::FileSize(1)))
        .set_compression(Some(CompressionMethod::Xz(10)));
    assert!(matches!(build_error(invalid), BuildError::InvalidXzCompressionLevel(10)));

    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn build_locks_the_log_path() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let builder = PipeLoggerBuilder::new(&path);
    let logger = builder.clone().build().unwrap();

    assert!(path.is_file());
    assert!(folder.join("logfile.log.pipe-logger.lock").is_file());
    assert!(matches!(build_error(builder.clone()), BuildError::LogPathAlreadyInUse(_)));

    drop(logger);
    builder.build().unwrap().finish().unwrap();
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn writes_text_lines_and_exact_bytes() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut logger = PipeLoggerBuilder::new(&path).build().unwrap();

    assert_eq!(None, logger.write_str("This is a log.").unwrap());
    assert_eq!(None, logger.write_line(" Another line.").unwrap());
    logger.write_all(&[0xFF, 0x00, 0x80]).unwrap();
    logger.finish().unwrap();

    assert_eq!(
        b"This is a log. Another line.\n\xFF\x00\x80".as_slice(),
        fs::read(&path).unwrap().as_slice()
    );

    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn writes_to_the_log_with_tee_enabled() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder.set_tee(Some(Tee::Stdout));
    let mut logger = builder.build().unwrap();

    logger.write_line("This is a log.").unwrap();
    logger.finish().unwrap();

    assert_eq!("This is a log.\n", fs::read_to_string(&path).unwrap());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn rotation_keeps_the_complete_line() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder.set_rotate(Some(RotateMethod::FileSize(24)));
    let mut logger = builder.build().unwrap();

    logger.write_line("This is a log.").unwrap();
    let rotated_path = logger.write_line("Isn't it?").unwrap().unwrap();
    logger.write_line("New file!").unwrap();
    logger.finish().unwrap();

    assert_eq!("New file!\n", fs::read_to_string(&path).unwrap());
    assert_eq!("This is a log.\nIsn't it?\n", fs::read_to_string(rotated_path).unwrap());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn count_includes_the_active_file() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder.set_rotate(Some(RotateMethod::FileSize(2))).set_count(Some(3));
    let mut logger = builder.build().unwrap();
    let mut rotated_paths = Vec::new();

    for text in ["a", "b", "c", "d"] {
        rotated_paths.push(logger.write_line(text).unwrap().unwrap());
    }
    logger.finish().unwrap();

    assert!(!rotated_paths[0].exists());
    assert!(!rotated_paths[1].exists());
    assert!(rotated_paths[2].exists());
    assert!(rotated_paths[3].exists());

    let count_one_folder = create_test_folder();
    let count_one_path = log_path(&count_one_folder);
    let mut builder = PipeLoggerBuilder::new(&count_one_path);
    builder.set_rotate(Some(RotateMethod::FileSize(1))).set_count(Some(1));
    let mut logger = builder.build().unwrap();
    assert_eq!(None, logger.write_line("discarded").unwrap());
    logger.finish().unwrap();
    assert_eq!(0, fs::metadata(&count_one_path).unwrap().len());

    fs::remove_dir_all(folder).unwrap();
    fs::remove_dir_all(count_one_folder).unwrap();
}

#[test]
fn scanner_accepts_exact_new_and_legacy_names_only() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder.set_rotate(Some(RotateMethod::FileSize(1)));
    let mut logger = builder.build().unwrap();
    let new_path = logger.write_line("new").unwrap().unwrap();
    logger.finish().unwrap();

    let legacy_path = folder.join("logfile-2020-02-29-12-34-56-789.log");
    let invalid_date_path = folder.join("logfile-2020-02-30-12-34-56-789.log");
    let unrelated_path = folder.join("logfile-2020-02-29-12-34-56-789-extra.log");
    fs::write(&legacy_path, b"legacy").unwrap();
    fs::write(&invalid_date_path, b"invalid").unwrap();
    fs::write(&unrelated_path, b"unrelated").unwrap();

    // macOS rejects non-UTF-8 file names, so only exercise this on other unix systems.
    #[cfg(all(unix, not(target_os = "macos")))]
    let non_utf8_path = {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let path = folder.join(OsString::from_vec(b"logfile-\xFF.log".to_vec()));
        fs::write(&path, b"non-utf8").unwrap();
        path
    };

    let mut builder = PipeLoggerBuilder::new(&path);
    builder.set_rotate(Some(RotateMethod::FileSize(1))).set_count(Some(2));
    builder.build().unwrap().finish().unwrap();

    assert!(!legacy_path.exists());
    assert!(new_path.exists());
    assert!(invalid_date_path.exists());
    assert!(unrelated_path.exists());
    #[cfg(all(unix, not(target_os = "macos")))]
    assert!(non_utf8_path.exists());

    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn finish_waits_for_xz_compression() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder
        .set_rotate(Some(RotateMethod::FileSize(1)))
        .set_compression(Some(CompressionMethod::Xz(6)));
    let mut logger = builder.build().unwrap();
    let compressed_path = logger.write_line("compressed log").unwrap().unwrap();

    logger.finish().unwrap();

    assert!(compressed_path.exists());
    assert_eq!(b"compressed log\n".as_slice(), read_xz(&compressed_path).as_slice());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn flush_waits_for_compression_and_count() {
    let folder = create_test_folder();
    let path = log_path(&folder);
    let mut builder = PipeLoggerBuilder::new(&path);
    builder
        .set_rotate(Some(RotateMethod::FileSize(2)))
        .set_count(Some(3))
        .set_compression(Some(CompressionMethod::Xz(6)));
    let mut logger = builder.build().unwrap();
    let mut compressed_paths = Vec::new();

    for text in ["a", "b", "c", "d"] {
        compressed_paths.push(logger.write_line(text).unwrap().unwrap());
    }
    logger.flush().unwrap();

    assert!(!compressed_paths[0].exists());
    assert!(!compressed_paths[1].exists());
    assert!(compressed_paths[2].exists());
    assert!(compressed_paths[3].exists());
    assert_eq!(b"d\n".as_slice(), read_xz(&compressed_paths[3]).as_slice());

    logger.finish().unwrap();
    fs::remove_dir_all(folder).unwrap();
}

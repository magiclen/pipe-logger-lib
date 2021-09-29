extern crate pipe_logger_lib;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use pipe_logger_lib::*;

const LOG_FILE_NAME: &str = "logfile.log";
const WAIT_DURATION_MILLI_SECONDS: u64 = 1000;
#[cfg(feature = "gzip")]
const FILEX_EXT: &str = ".gz";
#[cfg(feature = "xz")]
const FILEX_EXT: &str = ".xz";

static mut LAST_TEST_FOLDER_TIME: AtomicUsize = AtomicUsize::new(0);

fn create_test_folder() -> PathBuf {
    let test_folder_name =
        { unsafe { LAST_TEST_FOLDER_TIME.fetch_add(1, Ordering::SeqCst) }.to_string() };

    let folder = Path::new("tests").join("out").join(&test_folder_name);

    fs::create_dir_all(&folder).unwrap();

    folder
}

#[test]
fn build() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let builder = PipeLoggerBuilder::new(&test_log_path);

        builder.build().unwrap();
    }

    assert!(test_log_path.exists());

    assert!(test_log_path.is_file());

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let builder = PipeLoggerBuilder::new(&test_log_path);

        let mut logger = builder.build().unwrap();

        logger.write("This is a log.").unwrap();
    }

    let string = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("This is a log.", string);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_line() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let builder = PipeLoggerBuilder::new(&test_log_path);

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
    }

    let string = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("This is a log.\n", string);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_twice() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let builder = PipeLoggerBuilder::new(&test_log_path);

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write("Isn't it?").unwrap();
    }

    let string = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("This is a log.\nIsn't it?", string);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_tee_out() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_tee(Some(Tee::Stdout));

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap();
    }

    let string = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("This is a log.\nIsn't it?\n", string);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_tee_err() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_tee(Some(Tee::Stderr));

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap();
    }

    let string = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("This is a log.\nIsn't it?\n", string);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_rotate() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    let new_file = {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_rotate(Some(RotateMethod::FileSize(24)));

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        let new_file = logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("New file!!!!").unwrap();

        new_file
    };

    let string_1 = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("New file!!!!\n", string_1);

    let string_2 = fs::read_to_string(new_file).unwrap();

    assert_eq!("This is a log.\nIsn't it?", string_2);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_rotate_with_count() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    let new_file = {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_rotate(Some(RotateMethod::FileSize(24)));
        builder.set_count(Some(5));

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("This is a log.").unwrap();
        logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("This is a log.").unwrap();
        let new_file = logger.write_line("Isn't it?").unwrap().unwrap();

        logger.write_line("New file!!!!").unwrap();

        new_file
    };

    assert_eq!(5, test_folder.read_dir().unwrap().count());

    let string_1 = fs::read_to_string(test_log_path).unwrap();

    assert_eq!("New file!!!!\n", string_1);

    let string_2 = fs::read_to_string(new_file).unwrap();

    assert_eq!("This is a log.\nIsn't it?", string_2);

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_rotate_with_compress() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    let mut new_files = Vec::new();

    {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_rotate(Some(RotateMethod::FileSize(24)));
        builder.set_compress(true);

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("New file!!!!").unwrap();
    };

    thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS));

    if test_folder.read_dir().unwrap().count() != 7 {
        thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS * 2));
        if test_folder.read_dir().unwrap().count() != 7 {
            thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS * 3));
            assert_eq!(7, test_folder.read_dir().unwrap().count());
        }
    }

    for new_file in new_files {
        assert!(new_file.exists());

        assert!(new_file.to_str().unwrap().ends_with(FILEX_EXT));
    }

    fs::remove_dir_all(test_folder).unwrap();
}

#[test]
fn write_rotate_with_count_compress() {
    let test_folder = create_test_folder();

    let test_log_path = test_folder.join(LOG_FILE_NAME);

    let mut new_files = Vec::new();

    {
        let mut builder = PipeLoggerBuilder::new(&test_log_path);

        builder.set_rotate(Some(RotateMethod::FileSize(24)));
        builder.set_count(Some(5));
        builder.set_compress(true);

        let mut logger = builder.build().unwrap();

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("This is a log.").unwrap();
        new_files.push(logger.write_line("Isn't it?").unwrap().unwrap());

        logger.write_line("New file!!!!").unwrap();
    };

    thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS));

    if test_folder.read_dir().unwrap().count() != 5 {
        thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS * 2));
        if test_folder.read_dir().unwrap().count() != 5 {
            thread::sleep(Duration::from_millis(WAIT_DURATION_MILLI_SECONDS * 3));
            assert_eq!(5, test_folder.read_dir().unwrap().count());
        }
    }

    for new_file in new_files.iter().skip(2) {
        assert!(new_file.exists());

        assert!(new_file.to_str().unwrap().ends_with(FILEX_EXT));
    }

    fs::remove_dir_all(test_folder).unwrap();
}

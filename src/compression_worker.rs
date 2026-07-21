use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, Write},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};

use liblzma::write::XzEncoder;

use crate::rotated_file::{RotatedFile, enforce_retention, remove_if_exists};

enum Message {
    Rotate(RotatedFile),
    Barrier(SyncSender<io::Result<()>>),
    Shutdown(SyncSender<io::Result<()>>),
}

pub(crate) struct CompressionWorker {
    sender: SyncSender<Message>,
    handle: Option<JoinHandle<()>>,
}

impl CompressionWorker {
    pub(crate) fn start(
        rotated_files: VecDeque<RotatedFile>,
        max_rotated_files: usize,
        level: u32,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);

        let handle =
            thread::Builder::new().name("pipe-logger-compression".to_owned()).spawn(move || {
                run(receiver, rotated_files, max_rotated_files, level);
            })?;

        Ok(Self {
            sender,
            handle: Some(handle),
        })
    }

    pub(crate) fn rotate(&self, rotated_file: RotatedFile) -> io::Result<()> {
        self.sender.send(Message::Rotate(rotated_file)).map_err(channel_error)
    }

    pub(crate) fn barrier(&self) -> io::Result<()> {
        let (sender, receiver) = mpsc::sync_channel(0);

        self.sender.send(Message::Barrier(sender)).map_err(channel_error)?;

        receiver.recv().map_err(channel_error)?
    }

    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };

        let (sender, receiver) = mpsc::sync_channel(0);

        let send_result = self.sender.send(Message::Shutdown(sender)).map_err(channel_error);

        let worker_result = match send_result {
            Ok(()) => match receiver.recv() {
                Ok(result) => result,
                Err(error) => Err(channel_error(error)),
            },
            Err(error) => Err(error),
        };

        let join_result =
            handle.join().map_err(|_| io::Error::other("The compression worker panicked."));

        worker_result.and(join_result)
    }
}

fn run(
    receiver: Receiver<Message>,
    mut rotated_files: VecDeque<RotatedFile>,
    max_rotated_files: usize,
    level: u32,
) {
    let mut failed_compressions = VecDeque::new();
    let mut pending_error = None;

    // Drop files beyond the retention limit before compressing the survivors.
    if let Err(error) = enforce_retention(&mut rotated_files, max_rotated_files) {
        record_error(&mut pending_error, error);
    }

    if max_rotated_files > 0 {
        for rotated_file in rotated_files.iter() {
            if let Err(error) = compress_if_needed(rotated_file, level) {
                failed_compressions.push_back(rotated_file.clone());

                record_error(&mut pending_error, error);
            }
        }
    }

    while let Ok(message) = receiver.recv() {
        match message {
            Message::Rotate(rotated_file) => {
                if max_rotated_files > 0
                    && let Err(error) = compress_if_needed(&rotated_file, level)
                {
                    failed_compressions.push_back(rotated_file.clone());

                    record_error(&mut pending_error, error);
                }

                rotated_files.push_back(rotated_file);

                if let Err(error) = enforce_retention(&mut rotated_files, max_rotated_files) {
                    record_error(&mut pending_error, error);
                }
            },
            Message::Barrier(sender) => {
                if let Some(error) = retry_compressions(&mut failed_compressions, level) {
                    record_error(&mut pending_error, error);
                }

                if let Err(error) = enforce_retention(&mut rotated_files, max_rotated_files) {
                    record_error(&mut pending_error, error);
                }

                let _ = sender.send(take_result(&mut pending_error));
            },
            Message::Shutdown(sender) => {
                if let Some(error) = retry_compressions(&mut failed_compressions, level) {
                    record_error(&mut pending_error, error);
                }

                if let Err(error) = enforce_retention(&mut rotated_files, max_rotated_files) {
                    record_error(&mut pending_error, error);
                }

                let _ = sender.send(take_result(&mut pending_error));

                break;
            },
        }
    }
}

fn retry_compressions(
    failed_compressions: &mut VecDeque<RotatedFile>,
    level: u32,
) -> Option<io::Error> {
    let mut pending_error = None;
    let mut retry_again = VecDeque::new();

    while let Some(rotated_file) = failed_compressions.pop_front() {
        if let Err(error) = compress_if_needed(&rotated_file, level) {
            retry_again.push_back(rotated_file);

            record_error(&mut pending_error, error);
        }
    }

    *failed_compressions = retry_again;

    pending_error
}

fn compress_if_needed(rotated_file: &RotatedFile, level: u32) -> io::Result<()> {
    if !rotated_file.raw_path.try_exists()? {
        return Ok(());
    }

    if rotated_file.compressed_path.try_exists()? {
        return fs::remove_file(&rotated_file.raw_path);
    }

    compress(rotated_file, level)
}

fn compress(rotated_file: &RotatedFile, level: u32) -> io::Result<()> {
    let temporary_path = rotated_file.temporary_path();

    let result = (|| {
        remove_if_exists(&temporary_path)?;

        let mut input = File::open(&rotated_file.raw_path)?;

        let output = File::options().write(true).create_new(true).open(&temporary_path)?;

        let mut encoder = XzEncoder::new(output, level);

        io::copy(&mut input, &mut encoder)?;

        let mut output = encoder.finish()?;

        output.flush()?;

        drop(output);

        if rotated_file.compressed_path.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("`{}` already exists.", rotated_file.compressed_path.display()),
            ));
        }

        fs::rename(&temporary_path, &rotated_file.compressed_path)?;
        fs::remove_file(&rotated_file.raw_path)
    })();

    if result.is_err() {
        let _ = remove_if_exists(&temporary_path);
    }

    result
}

fn record_error(pending_error: &mut Option<io::Error>, error: io::Error) {
    if pending_error.is_none() {
        *pending_error = Some(error);
    }
}

fn take_result(pending_error: &mut Option<io::Error>) -> io::Result<()> {
    match pending_error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn channel_error<T>(error: T) -> io::Error
where
    T: std::fmt::Display, {
    io::Error::new(io::ErrorKind::BrokenPipe, error.to_string())
}

use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::{NaiveDateTime, Utc};

static ROTATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct RotatedFile {
    pub(crate) raw_path:        PathBuf,
    pub(crate) compressed_path: PathBuf,
    timestamp_nanos:            i64,
}

impl RotatedFile {
    pub(crate) fn temporary_path(&self) -> PathBuf {
        append_suffix(&self.compressed_path, ".tmp")
    }

    pub(crate) fn remove(&self) -> io::Result<()> {
        let raw_result = remove_if_exists(&self.raw_path);
        let compressed_result = remove_if_exists(&self.compressed_path);

        raw_result.and(compressed_result)
    }
}

pub(crate) fn create_rotated_file(
    folder_path: &Path,
    log_file_name: &OsStr,
) -> io::Result<RotatedFile> {
    loop {
        let now = Utc::now();

        let timestamp_nanos = now.timestamp_nanos_opt().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "The current UTC time is out of range.")
        })?;

        let sequence = ROTATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);

        let suffix = format!(
            "{}-{:09}-{:010}-{sequence:020}",
            now.format("%Y-%m-%d-%H-%M-%S"),
            now.timestamp_subsec_nanos(),
            process::id(),
        );

        let raw_path = folder_path.join(compose_rotated_file_name(log_file_name, &suffix)?);

        let rotated_file = RotatedFile {
            compressed_path: append_suffix(&raw_path, ".xz"),
            raw_path,
            timestamp_nanos,
        };

        if !rotated_file.raw_path.try_exists()?
            && !rotated_file.compressed_path.try_exists()?
            && !rotated_file.temporary_path().try_exists()?
        {
            return Ok(rotated_file);
        }
    }
}

pub(crate) fn scan_rotated_files(
    folder_path: &Path,
    log_file_name: &OsStr,
) -> io::Result<VecDeque<RotatedFile>> {
    let mut files = BTreeMap::new();

    for entry in folder_path.read_dir()? {
        let entry = entry?;

        if !entry.file_type()?.is_file() {
            continue;
        }

        let file_name = entry.file_name();

        if let Some(timestamp_nanos) = parse_rotated_file_name(&file_name, log_file_name) {
            let raw_path = entry.path();

            files.entry(raw_path.clone()).or_insert(RotatedFile {
                compressed_path: append_suffix(&raw_path, ".xz"),
                raw_path,
                timestamp_nanos,
            });

            continue;
        }

        if Path::new(&file_name).extension() != Some(OsStr::new("xz")) {
            continue;
        }

        let Some(raw_file_name) = Path::new(&file_name).file_stem() else {
            continue;
        };
        let Some(timestamp_nanos) = parse_rotated_file_name(raw_file_name, log_file_name) else {
            continue;
        };

        let raw_path = folder_path.join(raw_file_name);

        files.entry(raw_path.clone()).or_insert(RotatedFile {
            compressed_path: append_suffix(&raw_path, ".xz"),
            raw_path,
            timestamp_nanos,
        });
    }

    let mut files: Vec<_> = files.into_values().collect();
    files.sort_unstable_by(|a, b| {
        a.timestamp_nanos.cmp(&b.timestamp_nanos).then_with(|| a.raw_path.cmp(&b.raw_path))
    });

    Ok(files.into())
}

fn compose_rotated_file_name(log_file_name: &OsStr, suffix: &str) -> io::Result<OsString> {
    let log_path = Path::new(log_file_name);

    let stem = log_path.file_stem().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "The log path has no file stem.")
    })?;

    let mut file_name = stem.to_os_string();

    file_name.push("-");
    file_name.push(suffix);

    if let Some(extension) = log_path.extension() {
        file_name.push(".");
        file_name.push(extension);
    }

    Ok(file_name)
}

fn parse_rotated_file_name(file_name: &OsStr, log_file_name: &OsStr) -> Option<i64> {
    parse_new_rotated_file_name(file_name, log_file_name)
        .or_else(|| parse_legacy_rotated_file_name(file_name, log_file_name))
}

fn parse_new_rotated_file_name(file_name: &OsStr, log_file_name: &OsStr) -> Option<i64> {
    let file_path = Path::new(file_name);
    let log_path = Path::new(log_file_name);

    if file_path.extension() != log_path.extension() {
        return None;
    }

    let file_stem = file_path.file_stem()?.as_encoded_bytes();
    let log_stem = log_path.file_stem()?.as_encoded_bytes();

    if !file_stem.starts_with(log_stem) || file_stem.get(log_stem.len()) != Some(&b'-') {
        return None;
    }

    let suffix = std::str::from_utf8(&file_stem[log_stem.len() + 1..]).ok()?;

    // Fixed 61-byte layout: date (19) + '-' + subsec nanos (9) + '-' + pid (10) + '-' + sequence (20).
    if suffix.len() != 61
        || suffix.as_bytes().get(29) != Some(&b'-')
        || suffix.as_bytes().get(40) != Some(&b'-')
    {
        return None;
    }

    let timestamp = NaiveDateTime::parse_from_str(&suffix[..29], "%Y-%m-%d-%H-%M-%S-%f")
        .ok()?
        .and_utc()
        .timestamp_nanos_opt()?;

    suffix[30..40].parse::<u32>().ok()?;
    suffix[41..].parse::<u64>().ok()?;

    Some(timestamp)
}

fn parse_legacy_rotated_file_name(file_name: &OsStr, log_file_name: &OsStr) -> Option<i64> {
    let file_name = file_name.as_encoded_bytes();
    let log_file_name = log_file_name.as_encoded_bytes();

    let extension_index = log_file_name.iter().rposition(|byte| *byte == b'.');

    let (prefix, extension) = match extension_index {
        Some(index) => (&log_file_name[..index], &log_file_name[index..]),
        None => (log_file_name, &[][..]),
    };

    if file_name.len() != prefix.len() + 24 + extension.len()
        || !file_name.starts_with(prefix)
        || !file_name.ends_with(extension)
    {
        return None;
    }

    let suffix = std::str::from_utf8(&file_name[prefix.len()..prefix.len() + 24]).ok()?;

    if !suffix.starts_with('-') || suffix.as_bytes().get(20) != Some(&b'-') {
        return None;
    }

    let timestamp = NaiveDateTime::parse_from_str(&suffix[1..20], "%Y-%m-%d-%H-%M-%S")
        .ok()?
        .and_utc()
        .timestamp_nanos_opt()?;

    let milliseconds = suffix[21..].parse::<i64>().ok()?;

    timestamp.checked_add(milliseconds.checked_mul(1_000_000)?)
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();

    path.push(suffix);

    PathBuf::from(path)
}

pub(crate) fn enforce_retention(
    rotated_files: &mut VecDeque<RotatedFile>,
    max_rotated_files: usize,
) -> io::Result<()> {
    while rotated_files.len() > max_rotated_files {
        let Some(rotated_file) = rotated_files.front() else {
            break;
        };

        rotated_file.remove()?;

        rotated_files.pop_front();
    }

    Ok(())
}

pub(crate) fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

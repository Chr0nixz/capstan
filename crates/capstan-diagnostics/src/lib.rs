//! Bounded local diagnostic event log.
//!
//! An event holds only `&'static str` codes, numbers and flags, so hosts,
//! users, paths, credentials and terminal content cannot be recorded through
//! it. Codes that are not short lowercase tokens are written as `invalid`.
//!
//! Lines go to [`FILE_NAME`] in the directory given to [`install`]. When it
//! would pass [`MAX_FILE_BYTES`] it replaces [`PREVIOUS_FILE_NAME`], so the log
//! never holds more than two files. A writer thread owns the files; recording
//! never blocks, and an event is dropped while the queue is full or before
//! [`install`] runs.
use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Subdirectory of the application data root that holds the log.
pub const DIRECTORY: &str = "logs";
pub const FILE_NAME: &str = "diagnostics.log";
pub const PREVIOUS_FILE_NAME: &str = "diagnostics.1.log";
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
const QUEUE: usize = 256;
const MAX_FIELDS: usize = 8;
const MAX_TOKEN: usize = 64;

static SINK: OnceLock<mpsc::SyncSender<String>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    Code(&'static str),
    Number(u64),
    Flag(bool),
}

/// One log line, built with [`event`] and written by [`Event::record`].
#[must_use = "an event is written only by `record`"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    name: &'static str,
    fields: Vec<(&'static str, Value)>,
}

/// Start an event. `name` is a short lowercase token such as `connection`.
pub fn event(name: &'static str) -> Event {
    Event {
        name,
        fields: Vec::new(),
    }
}

impl Event {
    /// Add a static code, such as an error code or a phase name.
    pub fn code(self, key: &'static str, value: &'static str) -> Self {
        self.field(key, Value::Code(value))
    }

    pub fn number(self, key: &'static str, value: u64) -> Self {
        self.field(key, Value::Number(value))
    }

    /// Add a duration in whole milliseconds.
    pub fn millis(self, key: &'static str, value: Duration) -> Self {
        self.number(key, u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn flag(self, key: &'static str, value: bool) -> Self {
        self.field(key, Value::Flag(value))
    }

    fn field(mut self, key: &'static str, value: Value) -> Self {
        if self.fields.len() < MAX_FIELDS {
            self.fields.push((key, value));
        }
        self
    }

    /// Queue the event for the writer thread, if [`install`] has run.
    pub fn record(self) {
        if let Some(sink) = SINK.get() {
            let _ = sink.try_send(self.line(SystemTime::now()));
        }
    }

    fn line(&self, now: SystemTime) -> String {
        let mut line = format!(
            "{{\"time\":\"{}\",\"event\":\"{}\"",
            timestamp(now),
            token(self.name)
        );
        for (key, value) in &self.fields {
            let key = token(key);
            // Writing to a String cannot fail.
            let _ = match value {
                Value::Code(code) => write!(line, ",\"{key}\":\"{}\"", token(code)),
                Value::Number(number) => write!(line, ",\"{key}\":{number}"),
                Value::Flag(flag) => write!(line, ",\"{key}\":{flag}"),
            };
        }
        line.push_str("}\n");
        line
    }
}

/// Keeps every written string a JSON-safe token. A static string can still
/// hold anything, so anything else becomes `invalid`.
fn token(value: &str) -> &str {
    let valid = !value.is_empty()
        && value.len() <= MAX_TOKEN
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        });
    if valid {
        value
    } else {
        "invalid"
    }
}

/// UTC time with millisecond precision, for example `2026-11-21T08:05:03.042Z`.
fn timestamp(now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (year, month, day) = civil_date(seconds / 86_400);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3600,
        time / 60 % 60,
        time % 60,
        since.subsec_millis()
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`, restricted to dates after the epoch).
fn civil_date(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// Start the writer for `directory`, creating it if needed. Later calls do
/// nothing.
///
/// # Errors
///
/// Fails if the directory or log file cannot be created or opened, or the
/// writer thread cannot start. Events are then dropped.
pub fn install(directory: &Path) -> io::Result<()> {
    if SINK.get().is_some() {
        return Ok(());
    }
    fs::create_dir_all(directory)?;
    let mut log = Log::open(directory.to_owned(), MAX_FILE_BYTES)?;
    let (sender, receiver) = mpsc::sync_channel::<String>(QUEUE);
    std::thread::Builder::new()
        .name("capstan-diagnostics".into())
        .spawn(move || {
            for line in receiver {
                let _ = log.append(&line);
            }
        })?;
    // A concurrent install wins; this writer then sees its channel close.
    let _ = SINK.set(sender);
    Ok(())
}

struct Log {
    directory: PathBuf,
    limit: u64,
    file: Option<File>,
    size: u64,
}

impl Log {
    fn open(directory: PathBuf, limit: u64) -> io::Result<Self> {
        let mut log = Self {
            directory,
            limit,
            file: None,
            size: 0,
        };
        log.reopen()?;
        Ok(log)
    }

    fn reopen(&mut self) -> io::Result<()> {
        self.file = None;
        // The whole folder may have been deleted while running.
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join(FILE_NAME);
        // Appending through a planted link would write somewhere else.
        if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        self.size = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    fn append(&mut self, line: &str) -> io::Result<()> {
        let length = line.len() as u64;
        // Deleted while open: lines written through the old handle would
        // never be seen. Start a new file instead.
        if self.file.is_some() && fs::symlink_metadata(self.directory.join(FILE_NAME)).is_err() {
            self.file = None;
        }
        if self.file.is_none() {
            self.reopen()?;
        }
        if self.size.saturating_add(length) > self.limit {
            // Without a successful rotation the line is dropped, so the log
            // stays within two files.
            self.rotate()?;
        }
        let Some(file) = self.file.as_mut() else {
            return Err(io::ErrorKind::NotFound.into());
        };
        if let Err(error) = file.write_all(line.as_bytes()) {
            self.file = None;
            return Err(error);
        }
        self.size = self.size.saturating_add(length);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        match fs::rename(
            self.directory.join(FILE_NAME),
            self.directory.join(PREVIOUS_FILE_NAME),
        ) {
            Ok(()) => {}
            // Nothing to keep: the file was deleted. A new one starts below.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.reopen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_json_with_utc_time_and_only_token_codes() {
        let line = event("connection")
            .code("state", "reconnecting")
            .code("error", "connection_lost")
            .millis("after_ms", Duration::from_millis(1_234))
            .number("generation", 2)
            .flag("portable", false)
            .line(UNIX_EPOCH + Duration::from_millis(1_795_769_103_042));
        assert_eq!(
            line,
            "{\"time\":\"2026-11-27T08:45:03.042Z\",\"event\":\"connection\",\"state\":\"reconnecting\",\"error\":\"connection_lost\",\"after_ms\":1234,\"generation\":2,\"portable\":false}\n"
        );
        let leaked = event("Bad Name")
            .code("host", "user@example.test")
            .code("path", "C:\\Users\\me")
            .code("quote\"", "x\"y")
            .code("empty", "")
            .line(UNIX_EPOCH);
        assert_eq!(
            leaked,
            "{\"time\":\"1970-01-01T00:00:00.000Z\",\"event\":\"invalid\",\"host\":\"invalid\",\"path\":\"invalid\",\"invalid\":\"invalid\",\"empty\":\"invalid\"}\n"
        );
    }

    #[test]
    fn events_keep_a_bounded_number_of_fields() {
        let mut built = event("many");
        for number in 0..20 {
            built = built.number("n", number);
        }
        assert_eq!(built.fields.len(), MAX_FIELDS);
    }

    #[test]
    fn civil_dates_cover_leap_days_and_century_rules() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(11_017), (2000, 3, 1));
        assert_eq!(civil_date(47_540), (2100, 2, 28));
        assert_eq!(civil_date(47_541), (2100, 3, 1));
        assert_eq!(civil_date(20_778), (2026, 11, 21));
    }

    #[test]
    fn the_log_rotates_into_one_previous_file_and_stays_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut log = Log::open(root.path().to_owned(), 100).unwrap();
        let line = format!("{}\n", "x".repeat(39));
        for _ in 0..7 {
            log.append(&line).unwrap();
        }
        let current = fs::read_to_string(root.path().join(FILE_NAME)).unwrap();
        let previous = fs::read_to_string(root.path().join(PREVIOUS_FILE_NAME)).unwrap();
        assert_eq!(current.len(), 40);
        assert_eq!(previous.len(), 80);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);

        // A reopened log continues from the existing size.
        let mut reopened = Log::open(root.path().to_owned(), 100).unwrap();
        reopened.append(&line).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join(FILE_NAME))
                .unwrap()
                .len(),
            80
        );
    }

    #[test]
    fn a_log_deleted_while_running_starts_again() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("logs");
        fs::create_dir(&directory).unwrap();
        let mut log = Log::open(directory.clone(), 100).unwrap();
        let line = format!("{}\n", "x".repeat(39));
        log.append(&line).unwrap();
        fs::remove_file(directory.join(FILE_NAME)).unwrap();
        log.append(&line).unwrap();
        assert_eq!(fs::read_to_string(directory.join(FILE_NAME)).unwrap(), line);
        // Rotation keeps working afterwards.
        for _ in 0..3 {
            log.append(&line).unwrap();
        }
        assert_eq!(
            fs::read_to_string(directory.join(PREVIOUS_FILE_NAME))
                .unwrap()
                .len(),
            80
        );

        // So does deleting the whole folder.
        fs::remove_dir_all(&directory).unwrap();
        log.append(&line).unwrap();
        assert_eq!(fs::read_to_string(directory.join(FILE_NAME)).unwrap(), line);
    }

    #[test]
    fn rotation_with_nothing_to_rename_starts_a_new_file() {
        let root = tempfile::tempdir().unwrap();
        let mut log = Log::open(root.path().to_owned(), 100).unwrap();
        log.file = None;
        fs::remove_file(root.path().join(FILE_NAME)).unwrap();
        log.rotate().unwrap();
        assert!(root.path().join(FILE_NAME).is_file());
        assert!(!root.path().join(PREVIOUS_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_log_is_private_and_refuses_a_planted_link() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        Log::open(root.path().to_owned(), 100).unwrap();
        let mode = fs::metadata(root.path().join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        let planted = tempfile::tempdir().unwrap();
        let target = planted.path().join("target");
        fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(&target, planted.path().join(FILE_NAME)).unwrap();
        assert!(Log::open(planted.path().to_owned(), 100).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"");
    }
}

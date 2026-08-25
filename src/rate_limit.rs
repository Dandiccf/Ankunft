//! Persistent, conservative client-side limits for Parcel API attempts.
//!
//! A successful [`RateLimiter::reserve`] call means the attempt has already
//! been durably recorded. Callers must reserve first and only then start the
//! HTTP request. Reservations are intentionally never rolled back after a
//! network error or process crash.

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::i18n::{interpolate, tr};

const SCHEMA_VERSION: u32 = 1;
const STATE_DIRECTORY: &str = "io.github.dandiccf.Ankunft";
const MAX_STATE_BYTES: u64 = 64 * 1024;

pub const RATE_LIMIT_FILE_NAME: &str = "rate-limits-v1.json";
pub const READ_LIMIT: usize = 20;
pub const READ_WINDOW_SECS: u64 = 3_600;
pub const ADD_LIMIT: usize = 20;
pub const ADD_WINDOW_SECS: u64 = 86_400;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DEFAULT_LIMITER: OnceLock<Arc<RateLimiter>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Read,
    Add,
}

impl RequestKind {
    pub const fn limit(self) -> usize {
        match self {
            Self::Read => READ_LIMIT,
            Self::Add => ADD_LIMIT,
        }
    }

    pub const fn window_secs(self) -> u64 {
        match self {
            Self::Read => READ_WINDOW_SECS,
            Self::Add => ADD_WINDOW_SECS,
        }
    }

    pub fn localized_label(self) -> String {
        tr(match self {
            Self::Read => "Abruf",
            Self::Add => "Hinzufügen",
        })
    }
}

impl fmt::Display for RequestKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => formatter.write_str("Abruf"),
            Self::Add => formatter.write_str("Hinzufügen"),
        }
    }
}

/// Proof that an API attempt was durably counted before network I/O begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    pub kind: RequestKind,
    /// May be later than the current wall clock after a backwards adjustment.
    pub reserved_at_unix_secs: u64,
    pub remaining: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitStatus {
    pub kind: RequestKind,
    pub limit: usize,
    pub used: usize,
    pub remaining: usize,
    /// Present only while all slots in the rolling window are occupied.
    pub retry_at_unix_secs: Option<u64>,
    /// The non-decreasing time used for the calculation.
    pub observed_at_unix_secs: u64,
}

#[derive(Debug, Error)]
pub enum RateLimitError {
    #[error(
        "Das lokale Limit für {kind} ist erreicht. Der nächste Versuch ist ab Unix-Zeit {retry_at_unix_secs} möglich."
    )]
    LimitReached {
        kind: RequestKind,
        retry_at_unix_secs: u64,
    },
    #[error("Die Systemzeit liegt vor dem Unix-Zeitbeginn.")]
    ClockBeforeUnixEpoch,
    #[error("Der lokale API-Zähler konnte nicht gelesen oder geschrieben werden: {0}")]
    Io(#[from] io::Error),
    #[error("Der lokale API-Zähler ist beschädigt: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error(
        "Die Zähler-Version {found} wird von dieser Version von Ankunft nicht unterstützt (erwartet: {expected})."
    )]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("Der lokale API-Zähler ist inkonsistent: {0}")]
    InvalidState(&'static str),
    #[error("Der lokale API-Zähler ist ungewöhnlich groß ({size} Bytes).")]
    StateTooLarge { size: u64 },
    #[error("Der private Speicherpfad ist kein reguläres Verzeichnis: {0}")]
    InvalidDirectory(PathBuf),
    #[error("Die Zähler-Datei ist keine reguläre Datei: {0}")]
    InvalidFile(PathBuf),
    #[error("Der lokale API-Zähler ist intern gesperrt.")]
    LockPoisoned,
}

impl RateLimitError {
    pub fn localized_message(&self) -> String {
        match self {
            Self::LimitReached {
                kind,
                retry_at_unix_secs,
            } => interpolate(
                tr(
                    "Das lokale Limit für {kind} ist erreicht. Der nächste Versuch ist ab Unix-Zeit {retry_at_unix_secs} möglich.",
                ),
                &[
                    ("kind", &kind.localized_label()),
                    ("retry_at_unix_secs", &retry_at_unix_secs.to_string()),
                ],
            ),
            Self::ClockBeforeUnixEpoch => tr("Die Systemzeit liegt vor dem Unix-Zeitbeginn."),
            Self::Io(error) => {
                let error = error.to_string();
                interpolate(
                    tr("Der lokale API-Zähler konnte nicht gelesen oder geschrieben werden: {0}"),
                    &[("0", &error)],
                )
            }
            Self::InvalidJson(error) => {
                let error = error.to_string();
                interpolate(
                    tr("Der lokale API-Zähler ist beschädigt: {0}"),
                    &[("0", &error)],
                )
            }
            Self::UnsupportedVersion { found, expected } => interpolate(
                tr(
                    "Die Zähler-Version {found} wird von dieser Version von Ankunft nicht unterstützt (erwartet: {expected}).",
                ),
                &[
                    ("found", &found.to_string()),
                    ("expected", &expected.to_string()),
                ],
            ),
            Self::InvalidState(error) => interpolate(
                tr("Der lokale API-Zähler ist inkonsistent: {0}"),
                &[("0", error)],
            ),
            Self::StateTooLarge { size } => interpolate(
                tr("Der lokale API-Zähler ist ungewöhnlich groß ({size} Bytes)."),
                &[("size", &size.to_string())],
            ),
            Self::InvalidDirectory(path) => interpolate(
                tr("Der private Speicherpfad ist kein reguläres Verzeichnis: {0}"),
                &[("0", &path.display().to_string())],
            ),
            Self::InvalidFile(path) => interpolate(
                tr("Die Zähler-Datei ist keine reguläre Datei: {0}"),
                &[("0", &path.display().to_string())],
            ),
            Self::LockPoisoned => tr("Der lokale API-Zähler ist intern gesperrt."),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RateLimitDocument {
    schema_version: u32,
    #[serde(default)]
    last_observed_unix_secs: Option<u64>,
    #[serde(default)]
    read_attempts_unix_secs: Vec<u64>,
    #[serde(default)]
    add_attempts_unix_secs: Vec<u64>,
}

impl Default for RateLimitDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            last_observed_unix_secs: None,
            read_attempts_unix_secs: Vec::new(),
            add_attempts_unix_secs: Vec::new(),
        }
    }
}

impl RateLimitDocument {
    fn attempts(&self, kind: RequestKind) -> &[u64] {
        match kind {
            RequestKind::Read => &self.read_attempts_unix_secs,
            RequestKind::Add => &self.add_attempts_unix_secs,
        }
    }

    fn attempts_mut(&mut self, kind: RequestKind) -> &mut Vec<u64> {
        match kind {
            RequestKind::Read => &mut self.read_attempts_unix_secs,
            RequestKind::Add => &mut self.add_attempts_unix_secs,
        }
    }

    /// Wall time is allowed to advance but never to move backwards. If NTP or
    /// the user turns the clock back, existing attempts therefore remain in
    /// their windows instead of being incorrectly discarded.
    fn effective_now(&self, wall_now_unix_secs: u64) -> u64 {
        self.last_observed_unix_secs
            .unwrap_or(wall_now_unix_secs)
            .max(wall_now_unix_secs)
    }
}

/// Thread-safe persistent rolling-window limiter.
///
/// This is an in-process lock. It is sufficient for the single-instance GNOME
/// application; a future independent daemon would additionally need an
/// inter-process lock around the read-modify-write transaction.
pub struct RateLimiter {
    path: PathBuf,
    state: Mutex<RateLimitDocument>,
}

impl RateLimiter {
    /// Returns the one limiter shared by all API clients in this process.
    pub fn shared_default() -> Result<Arc<Self>, RateLimitError> {
        if let Some(limiter) = DEFAULT_LIMITER.get() {
            return Ok(Arc::clone(limiter));
        }

        let limiter = Arc::new(Self::open_default()?);
        match DEFAULT_LIMITER.set(Arc::clone(&limiter)) {
            Ok(()) => Ok(limiter),
            Err(_) => {
                Ok(Arc::clone(DEFAULT_LIMITER.get().expect(
                    "default limiter initialized by a concurrent caller",
                )))
            }
        }
    }

    pub fn open_default() -> Result<Self, RateLimitError> {
        Self::open_in(user_state_directory().join(STATE_DIRECTORY))
    }

    /// Opens a limiter inside `directory`; useful for tests and sandbox roots.
    pub fn open_in(directory: impl AsRef<Path>) -> Result<Self, RateLimitError> {
        let directory = directory.as_ref();
        ensure_private_directory(directory)?;

        let path = directory.join(RATE_LIMIT_FILE_NAME);
        let state = load_document(&path)?;

        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Durably consumes one slot and returns only after the atomic state write.
    /// The caller may start the corresponding HTTP request only after `Ok`.
    pub fn reserve(&self, kind: RequestKind) -> Result<Reservation, RateLimitError> {
        self.reserve_at(kind, unix_now()?)
    }

    pub fn status(&self, kind: RequestKind) -> Result<RateLimitStatus, RateLimitError> {
        self.status_at(kind, unix_now()?)
    }

    fn reserve_at(
        &self,
        kind: RequestKind,
        wall_now_unix_secs: u64,
    ) -> Result<Reservation, RateLimitError> {
        let mut state = self.lock_state()?;
        let mut next = state.clone();
        let effective_now = next.effective_now(wall_now_unix_secs);
        let limit = kind.limit();
        let window_secs = kind.window_secs();

        let remaining = {
            let attempts = next.attempts_mut(kind);
            prune_expired(attempts, effective_now, window_secs);

            if attempts.len() >= limit {
                return Err(RateLimitError::LimitReached {
                    kind,
                    retry_at_unix_secs: retry_at(attempts, window_secs),
                });
            }

            attempts.push(effective_now);
            limit - attempts.len()
        };
        next.last_observed_unix_secs = Some(effective_now);

        let bytes = serde_json::to_vec(&next)?;
        match write_atomically(&self.path, &bytes) {
            Ok(()) => *state = next,
            Err(failure) if failure.renamed => {
                // The complete new file is already visible; only the directory
                // fsync failed. Keep memory aligned while reporting uncertainty.
                *state = next;
                return Err(RateLimitError::Io(failure.source));
            }
            Err(failure) => return Err(RateLimitError::Io(failure.source)),
        }

        Ok(Reservation {
            kind,
            reserved_at_unix_secs: effective_now,
            remaining,
        })
    }

    fn status_at(
        &self,
        kind: RequestKind,
        wall_now_unix_secs: u64,
    ) -> Result<RateLimitStatus, RateLimitError> {
        let state = self.lock_state()?;
        let effective_now = state.effective_now(wall_now_unix_secs);
        let mut attempts = state.attempts(kind).to_vec();
        prune_expired(&mut attempts, effective_now, kind.window_secs());

        let used = attempts.len().min(kind.limit());
        Ok(RateLimitStatus {
            kind,
            limit: kind.limit(),
            used,
            remaining: kind.limit() - used,
            retry_at_unix_secs: (attempts.len() >= kind.limit())
                .then(|| retry_at(&attempts, kind.window_secs())),
            observed_at_unix_secs: effective_now,
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, RateLimitDocument>, RateLimitError> {
        self.state.lock().map_err(|_| RateLimitError::LockPoisoned)
    }
}

fn unix_now() -> Result<u64, RateLimitError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| RateLimitError::ClockBeforeUnixEpoch)
}

fn prune_expired(attempts: &mut Vec<u64>, now: u64, window_secs: u64) {
    attempts.retain(|timestamp| now.saturating_sub(*timestamp) < window_secs);
}

fn retry_at(attempts: &[u64], window_secs: u64) -> u64 {
    attempts
        .iter()
        .copied()
        .min()
        .unwrap_or(0)
        .saturating_add(window_secs)
}

fn user_state_directory() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state"))
}

fn load_document(path: &Path) -> Result<RateLimitDocument, RateLimitError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RateLimitDocument::default());
        }
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_file() {
        return Err(RateLimitError::InvalidFile(path.to_path_buf()));
    }
    if metadata.len() > MAX_STATE_BYTES {
        return Err(RateLimitError::StateTooLarge {
            size: metadata.len(),
        });
    }

    make_file_private(path)?;

    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(RateLimitError::StateTooLarge {
            size: bytes.len() as u64,
        });
    }

    let document: RateLimitDocument = serde_json::from_slice(&bytes)?;
    if document.schema_version != SCHEMA_VERSION {
        return Err(RateLimitError::UnsupportedVersion {
            found: document.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    validate_document(&document)?;

    Ok(document)
}

fn validate_document(document: &RateLimitDocument) -> Result<(), RateLimitError> {
    if document.read_attempts_unix_secs.len() > READ_LIMIT {
        return Err(RateLimitError::InvalidState(
            "zu viele gespeicherte Leseversuche",
        ));
    }
    if document.add_attempts_unix_secs.len() > ADD_LIMIT {
        return Err(RateLimitError::InvalidState(
            "zu viele gespeicherte Hinzufügeversuche",
        ));
    }

    let attempts_exist =
        !document.read_attempts_unix_secs.is_empty() || !document.add_attempts_unix_secs.is_empty();
    let Some(last_observed) = document.last_observed_unix_secs else {
        return if attempts_exist {
            Err(RateLimitError::InvalidState(
                "Versuche ohne letzten Zeitbezug",
            ))
        } else {
            Ok(())
        };
    };

    for attempts in [
        &document.read_attempts_unix_secs,
        &document.add_attempts_unix_secs,
    ] {
        if !attempts.windows(2).all(|pair| pair[0] <= pair[1]) {
            return Err(RateLimitError::InvalidState(
                "Zeitstempel sind nicht sortiert",
            ));
        }
        if attempts.iter().any(|timestamp| *timestamp > last_observed) {
            return Err(RateLimitError::InvalidState(
                "Versuch liegt nach der zuletzt beobachteten Zeit",
            ));
        }
    }

    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), RateLimitError> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(RateLimitError::InvalidDirectory(path.to_path_buf()));
    }

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;

    Ok(())
}

fn make_file_private(path: &Path) -> Result<(), RateLimitError> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;

    Ok(())
}

struct AtomicWriteFailure {
    source: io::Error,
    /// True once `rename` made the complete new state visible.
    renamed: bool,
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), AtomicWriteFailure> {
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(AtomicWriteFailure {
            source: io::Error::new(io::ErrorKind::InvalidData, "rate-limit state is too large"),
            renamed: false,
        });
    }

    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Err(error) = ensure_private_directory(parent) {
        return Err(AtomicWriteFailure {
            source: rate_limit_error_to_io(error),
            renamed: false,
        });
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(AtomicWriteFailure {
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("rate-limit path is not a regular file: {}", path.display()),
                ),
                renamed: false,
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AtomicWriteFailure {
                source: error,
                renamed: false,
            });
        }
    }

    let (temporary_path, mut temporary_file) = create_private_temporary_file(parent, path)
        .map_err(|source| AtomicWriteFailure {
            source,
            renamed: false,
        })?;
    let temporary_guard = TemporaryPath(temporary_path.clone());

    temporary_file
        .write_all(bytes)
        .and_then(|_| temporary_file.sync_all())
        .map_err(|source| AtomicWriteFailure {
            source,
            renamed: false,
        })?;
    drop(temporary_file);

    fs::rename(&temporary_path, path).map_err(|source| AtomicWriteFailure {
        source,
        renamed: false,
    })?;
    drop(temporary_guard);

    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| AtomicWriteFailure {
            source,
            renamed: true,
        })?;

    Ok(())
}

fn create_private_temporary_file(parent: &Path, destination: &Path) -> io::Result<(PathBuf, File)> {
    let stem = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rate-limits");

    for _ in 0..64 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{stem}.tmp-{}-{sequence}", process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);

        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique rate-limit temporary file",
    ))
}

fn rate_limit_error_to_io(error: RateLimitError) -> io::Error {
    match error {
        RateLimitError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
    }
}

struct TemporaryPath(PathBuf);

impl Drop for TemporaryPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, sync::Arc, thread};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            Self(env::temp_dir().join(format!(
                "ankunft-rate-limit-{label}-{}-{sequence}",
                process::id()
            )))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn blocks_the_twenty_first_read_attempt_in_a_rolling_hour() {
        let directory = TestDirectory::new("read-limit");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");

        for remaining in (0..READ_LIMIT).rev() {
            let reservation = limiter
                .reserve_at(RequestKind::Read, 1_000)
                .expect("reserve read");
            assert_eq!(reservation.remaining, remaining);
        }

        assert!(matches!(
            limiter.reserve_at(RequestKind::Read, 1_001),
            Err(RateLimitError::LimitReached {
                kind: RequestKind::Read,
                retry_at_unix_secs: 4_600
            })
        ));
    }

    #[test]
    fn a_slot_reopens_exactly_at_the_window_boundary() {
        let directory = TestDirectory::new("boundary");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        for _ in 0..READ_LIMIT {
            limiter
                .reserve_at(RequestKind::Read, 1_000)
                .expect("fill read window");
        }

        assert!(matches!(
            limiter.reserve_at(RequestKind::Read, 4_599),
            Err(RateLimitError::LimitReached { .. })
        ));
        let reservation = limiter
            .reserve_at(RequestKind::Read, 4_600)
            .expect("expired attempts must free slots");
        assert_eq!(reservation.remaining, READ_LIMIT - 1);
    }

    #[test]
    fn read_and_add_windows_are_independent() {
        let directory = TestDirectory::new("independent");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        for _ in 0..READ_LIMIT {
            limiter
                .reserve_at(RequestKind::Read, 10_000)
                .expect("fill reads");
        }

        let add = limiter
            .reserve_at(RequestKind::Add, 10_000)
            .expect("add has its own bucket");
        assert_eq!(add.remaining, ADD_LIMIT - 1);
        assert_eq!(
            limiter
                .status_at(RequestKind::Read, 10_000)
                .expect("read status")
                .remaining,
            0
        );
    }

    #[test]
    fn add_attempts_use_a_rolling_twenty_four_hour_window() {
        let directory = TestDirectory::new("add-window");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        for _ in 0..ADD_LIMIT {
            limiter
                .reserve_at(RequestKind::Add, 5_000)
                .expect("fill add window");
        }

        assert!(matches!(
            limiter.reserve_at(RequestKind::Add, 5_000 + ADD_WINDOW_SECS - 1),
            Err(RateLimitError::LimitReached { .. })
        ));
        assert!(
            limiter
                .reserve_at(RequestKind::Add, 5_000 + ADD_WINDOW_SECS)
                .is_ok()
        );
    }

    #[test]
    fn reservation_is_persistent_before_the_caller_continues() {
        let directory = TestDirectory::new("restart");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        limiter
            .reserve_at(RequestKind::Read, 20_000)
            .expect("durable reservation");
        drop(limiter);

        let reopened = RateLimiter::open_in(&directory.0).expect("reopen limiter");
        let status = reopened
            .status_at(RequestKind::Read, 20_000)
            .expect("read status");
        assert_eq!(status.used, 1);
        assert_eq!(status.remaining, READ_LIMIT - 1);
    }

    #[test]
    fn a_failed_state_write_does_not_grant_or_consume_a_slot() {
        let directory = TestDirectory::new("write-failure");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");

        // A directory at the destination cannot be atomically replaced as the
        // regular state file and deterministically simulates a commit failure.
        fs::create_dir(limiter.path()).expect("block state destination");
        assert!(matches!(
            limiter.reserve_at(RequestKind::Read, 20_000),
            Err(RateLimitError::Io(_))
        ));
        fs::remove_dir(limiter.path()).expect("unblock state destination");

        let reservation = limiter
            .reserve_at(RequestKind::Read, 20_000)
            .expect("first successful reservation");
        assert_eq!(reservation.remaining, READ_LIMIT - 1);
    }

    #[test]
    fn backwards_clock_adjustment_never_expires_attempts() {
        let directory = TestDirectory::new("clock-rollback");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        let first = limiter
            .reserve_at(RequestKind::Read, 10_000)
            .expect("first reservation");
        let after_rollback = limiter
            .reserve_at(RequestKind::Read, 5_000)
            .expect("conservative reservation after rollback");

        assert_eq!(first.reserved_at_unix_secs, 10_000);
        assert_eq!(after_rollback.reserved_at_unix_secs, 10_000);
        assert_eq!(
            limiter
                .status_at(RequestKind::Read, 5_000)
                .expect("status after rollback")
                .used,
            2
        );
    }

    #[test]
    fn concurrent_reservations_cannot_exceed_the_limit() {
        let directory = TestDirectory::new("concurrent");
        let limiter = Arc::new(RateLimiter::open_in(&directory.0).expect("open limiter"));
        let handles: Vec<_> = (0..32)
            .map(|_| {
                let limiter = Arc::clone(&limiter);
                thread::spawn(move || limiter.reserve_at(RequestKind::Read, 30_000).is_ok())
            })
            .collect();

        let accepted = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker did not panic"))
            .filter(|accepted| *accepted)
            .count();
        assert_eq!(accepted, READ_LIMIT);
    }

    #[test]
    fn truncated_or_semantically_invalid_state_is_not_reset() {
        let truncated_directory = TestDirectory::new("truncated");
        ensure_private_directory(&truncated_directory.0).expect("create directory");
        fs::write(
            truncated_directory.0.join(RATE_LIMIT_FILE_NAME),
            b"{\"schema_version\":1",
        )
        .expect("write truncated state");
        assert!(matches!(
            RateLimiter::open_in(&truncated_directory.0),
            Err(RateLimitError::InvalidJson(_))
        ));

        let invalid_directory = TestDirectory::new("invalid");
        ensure_private_directory(&invalid_directory.0).expect("create directory");
        fs::write(
            invalid_directory.0.join(RATE_LIMIT_FILE_NAME),
            br#"{"schema_version":1,"last_observed_unix_secs":100,"read_attempts_unix_secs":[101],"add_attempts_unix_secs":[]}"#,
        )
        .expect("write inconsistent state");
        assert!(matches!(
            RateLimiter::open_in(&invalid_directory.0),
            Err(RateLimitError::InvalidState(_))
        ));
    }

    #[test]
    fn rejects_unknown_schema_versions() {
        let directory = TestDirectory::new("schema");
        ensure_private_directory(&directory.0).expect("create directory");
        fs::write(
            directory.0.join(RATE_LIMIT_FILE_NAME),
            br#"{"schema_version":2,"last_observed_unix_secs":null,"read_attempts_unix_secs":[],"add_attempts_unix_secs":[]}"#,
        )
        .expect("write future state");

        assert!(matches!(
            RateLimiter::open_in(&directory.0),
            Err(RateLimitError::UnsupportedVersion {
                found: 2,
                expected: SCHEMA_VERSION
            })
        ));
    }

    #[test]
    fn successful_writes_leave_only_the_versioned_state_file() {
        let directory = TestDirectory::new("temporary");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        limiter
            .reserve_at(RequestKind::Read, 40_000)
            .expect("reserve read");

        let names: Vec<_> = fs::read_dir(&directory.0)
            .expect("read directory")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(RATE_LIMIT_FILE_NAME)]);
    }

    #[cfg(unix)]
    #[test]
    fn state_directory_and_file_are_private() {
        let directory = TestDirectory::new("permissions");
        let limiter = RateLimiter::open_in(&directory.0).expect("open limiter");
        limiter
            .reserve_at(RequestKind::Read, 50_000)
            .expect("reserve read");

        let directory_mode = fs::metadata(&directory.0)
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(limiter.path())
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_as_the_state_file() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("symlink");
        ensure_private_directory(&directory.0).expect("create directory");
        let unrelated = directory.0.join("unrelated.json");
        fs::write(&unrelated, b"do not touch").expect("write unrelated file");
        symlink(&unrelated, directory.0.join(RATE_LIMIT_FILE_NAME)).expect("create symlink");

        assert!(matches!(
            RateLimiter::open_in(&directory.0),
            Err(RateLimitError::InvalidFile(_))
        ));
        assert_eq!(
            fs::read(&unrelated).expect("read unrelated file"),
            b"do not touch"
        );
    }
}

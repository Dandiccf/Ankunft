//! Private, crash-resistant persistence for normalized delivery snapshots.
//!
//! The cache deliberately accepts only the application's domain model. Raw
//! Parcel responses and credentials therefore cannot accidentally be persisted
//! through this API.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    i18n::{interpolate, tr},
    model::Delivery,
};

const SCHEMA_VERSION: u32 = 1;
const CACHE_DIRECTORY: &str = "io.github.dandiccf.Ankunft";
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
pub const CACHE_FILE_NAME: &str = "deliveries-v1.json";

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DEFAULT_CACHE: OnceLock<Arc<DeliveryCache>> = OnceLock::new();

/// The Parcel API view represented by a cached snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Active,
    Recent,
}

/// The latest successfully fetched deliveries for one API view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliverySnapshot {
    /// UTC Unix time at which the complete response was accepted.
    pub fetched_at_unix_secs: u64,
    /// Normalized domain objects, never the raw Parcel response.
    pub deliveries: Vec<Delivery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheDocument {
    schema_version: u32,
    #[serde(default)]
    active: Option<DeliverySnapshot>,
    #[serde(default)]
    recent: Option<DeliverySnapshot>,
}

impl Default for CacheDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            active: None,
            recent: None,
        }
    }
}

impl CacheDocument {
    fn snapshot(&self, kind: SnapshotKind) -> Option<&DeliverySnapshot> {
        match kind {
            SnapshotKind::Active => self.active.as_ref(),
            SnapshotKind::Recent => self.recent.as_ref(),
        }
    }

    fn replace(
        &mut self,
        kind: SnapshotKind,
        snapshot: DeliverySnapshot,
    ) -> Option<DeliverySnapshot> {
        match kind {
            SnapshotKind::Active => self.active.replace(snapshot),
            SnapshotKind::Recent => self.recent.replace(snapshot),
        }
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Der lokale Sendungsspeicher konnte nicht gelesen oder geschrieben werden: {0}")]
    Io(#[from] io::Error),
    #[error("Der lokale Sendungsspeicher ist beschädigt: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error(
        "Die Cache-Version {found} wird von dieser Version von Ankunft nicht unterstützt (erwartet: {expected})."
    )]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("Der lokale Sendungsspeicher ist ungewöhnlich groß ({size} Bytes).")]
    CacheTooLarge { size: u64 },
    #[error("Der private Speicherpfad ist kein reguläres Verzeichnis: {0}")]
    InvalidDirectory(PathBuf),
    #[error("Die Cache-Datei ist keine reguläre Datei: {0}")]
    InvalidFile(PathBuf),
    #[error("Der lokale Sendungsspeicher ist intern gesperrt.")]
    LockPoisoned,
}

impl StorageError {
    pub fn localized_message(&self) -> String {
        match self {
            Self::Io(error) => {
                let error = error.to_string();
                interpolate(
                    tr(
                        "Der lokale Sendungsspeicher konnte nicht gelesen oder geschrieben werden: {0}",
                    ),
                    &[("0", &error)],
                )
            }
            Self::InvalidJson(error) => {
                let error = error.to_string();
                interpolate(
                    tr("Der lokale Sendungsspeicher ist beschädigt: {0}"),
                    &[("0", &error)],
                )
            }
            Self::UnsupportedVersion { found, expected } => interpolate(
                tr(
                    "Die Cache-Version {found} wird von dieser Version von Ankunft nicht unterstützt (erwartet: {expected}).",
                ),
                &[
                    ("found", &found.to_string()),
                    ("expected", &expected.to_string()),
                ],
            ),
            Self::CacheTooLarge { size } => interpolate(
                tr("Der lokale Sendungsspeicher ist ungewöhnlich groß ({size} Bytes)."),
                &[("size", &size.to_string())],
            ),
            Self::InvalidDirectory(path) => interpolate(
                tr("Der private Speicherpfad ist kein reguläres Verzeichnis: {0}"),
                &[("0", &path.display().to_string())],
            ),
            Self::InvalidFile(path) => interpolate(
                tr("Die Cache-Datei ist keine reguläre Datei: {0}"),
                &[("0", &path.display().to_string())],
            ),
            Self::LockPoisoned => tr("Der lokale Sendungsspeicher ist intern gesperrt."),
        }
    }
}

/// Thread-safe owner of the latest active and recent delivery snapshots.
///
/// `DeliveryCache` keeps one in-memory document and serializes updates through
/// a mutex. GNOME's single-instance application model ensures that there is a
/// single writer process; a future standalone background daemon would require
/// an additional inter-process lock.
pub struct DeliveryCache {
    path: PathBuf,
    state: Mutex<CacheDocument>,
}

impl DeliveryCache {
    /// Returns the one cache instance shared by every window and sync task in
    /// this application process.
    pub fn shared_default() -> Result<Arc<Self>, StorageError> {
        if let Some(cache) = DEFAULT_CACHE.get() {
            return Ok(Arc::clone(cache));
        }

        let cache = Arc::new(Self::open_default()?);
        match DEFAULT_CACHE.set(Arc::clone(&cache)) {
            Ok(()) => Ok(cache),
            Err(_) => {
                Ok(Arc::clone(DEFAULT_CACHE.get().expect(
                    "default cache initialized by a concurrent caller",
                )))
            }
        }
    }

    /// Opens the per-user cache below the XDG state directory.
    pub fn open_default() -> Result<Self, StorageError> {
        Self::open_in(user_state_directory().join(CACHE_DIRECTORY))
    }

    /// Opens a cache inside `directory`. Primarily useful for tests and future
    /// sandbox-specific storage roots.
    pub fn open_in(directory: impl AsRef<Path>) -> Result<Self, StorageError> {
        let directory = directory.as_ref();
        ensure_private_directory(directory)?;

        let path = directory.join(CACHE_FILE_NAME);
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

    /// Returns an owned snapshot so callers never hold the persistence lock.
    pub fn snapshot(&self, kind: SnapshotKind) -> Result<Option<DeliverySnapshot>, StorageError> {
        Ok(self.lock_state()?.snapshot(kind).cloned())
    }

    /// Atomically replaces one API view and returns its previous snapshot.
    ///
    /// The other view is retained. The on-disk file is committed before the
    /// new value becomes visible through this instance.
    pub fn replace_snapshot(
        &self,
        kind: SnapshotKind,
        fetched_at_unix_secs: u64,
        deliveries: Vec<Delivery>,
    ) -> Result<Option<DeliverySnapshot>, StorageError> {
        let mut state = self.lock_state()?;
        let mut next = state.clone();
        let previous = next.replace(
            kind,
            DeliverySnapshot {
                fetched_at_unix_secs,
                deliveries,
            },
        );

        let bytes = serde_json::to_vec(&next)?;
        match write_atomically(&self.path, &bytes) {
            Ok(()) => *state = next,
            Err(failure) if failure.renamed => {
                // The new file is already visible. A directory fsync failure
                // only means its crash durability is uncertain, so keeping the
                // in-memory state aligned with the visible file is safest.
                *state = next;
                return Err(StorageError::Io(failure.source));
            }
            Err(failure) => return Err(StorageError::Io(failure.source)),
        }

        Ok(previous)
    }

    /// Removes all cached deliveries while keeping an empty, valid envelope.
    pub fn clear(&self) -> Result<(), StorageError> {
        let mut state = self.lock_state()?;
        let next = CacheDocument::default();
        let bytes = serde_json::to_vec(&next)?;

        match write_atomically(&self.path, &bytes) {
            Ok(()) => *state = next,
            Err(failure) if failure.renamed => {
                *state = next;
                return Err(StorageError::Io(failure.source));
            }
            Err(failure) => return Err(StorageError::Io(failure.source)),
        }

        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, CacheDocument>, StorageError> {
        self.state.lock().map_err(|_| StorageError::LockPoisoned)
    }
}

fn user_state_directory() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state"))
}

fn load_document(path: &Path) -> Result<CacheDocument, StorageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CacheDocument::default());
        }
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_file() {
        return Err(StorageError::InvalidFile(path.to_path_buf()));
    }
    if metadata.len() > MAX_CACHE_BYTES {
        return Err(StorageError::CacheTooLarge {
            size: metadata.len(),
        });
    }

    make_file_private(path)?;

    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(StorageError::CacheTooLarge {
            size: bytes.len() as u64,
        });
    }

    let document: CacheDocument = serde_json::from_slice(&bytes)?;
    if document.schema_version != SCHEMA_VERSION {
        return Err(StorageError::UnsupportedVersion {
            found: document.schema_version,
            expected: SCHEMA_VERSION,
        });
    }

    Ok(document)
}

fn ensure_private_directory(path: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(StorageError::InvalidDirectory(path.to_path_buf()));
    }

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;

    Ok(())
}

fn make_file_private(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;

    Ok(())
}

struct AtomicWriteFailure {
    source: io::Error,
    /// True once `rename` made the complete new file visible.
    renamed: bool,
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), AtomicWriteFailure> {
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(AtomicWriteFailure {
            source: io::Error::new(io::ErrorKind::InvalidData, "cache exceeds size limit"),
            renamed: false,
        });
    }

    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Err(error) = ensure_private_directory(parent) {
        return Err(AtomicWriteFailure {
            source: storage_error_to_io(error),
            renamed: false,
        });
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(AtomicWriteFailure {
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("cache path is not a regular file: {}", path.display()),
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

    // The guard now points to a non-existent path and is harmless. Keeping it
    // alive until after the rename ensures every earlier error removes a temp.
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
        .unwrap_or("deliveries");

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
        "could not allocate a unique cache temporary file",
    ))
}

fn storage_error_to_io(error: StorageError) -> io::Error {
    match error {
        StorageError::Io(error) => error,
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
    use std::{env, sync::atomic::AtomicU64};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            Self(env::temp_dir().join(format!(
                "ankunft-storage-{label}-{}-{sequence}",
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
    fn snapshots_survive_restart_and_views_remain_independent() {
        let directory = TestDirectory::new("roundtrip");
        let cache = DeliveryCache::open_in(&directory.0).expect("open cache");
        let deliveries = crate::model::demo_deliveries();

        cache
            .replace_snapshot(
                SnapshotKind::Active,
                1_800_000_000,
                deliveries[..2].to_vec(),
            )
            .expect("write active snapshot");
        cache
            .replace_snapshot(
                SnapshotKind::Recent,
                1_800_000_100,
                deliveries[4..].to_vec(),
            )
            .expect("write recent snapshot");
        drop(cache);

        let reopened = DeliveryCache::open_in(&directory.0).expect("reopen cache");
        let active = reopened
            .snapshot(SnapshotKind::Active)
            .expect("read active")
            .expect("active snapshot");
        let recent = reopened
            .snapshot(SnapshotKind::Recent)
            .expect("read recent")
            .expect("recent snapshot");

        assert_eq!(active.fetched_at_unix_secs, 1_800_000_000);
        assert_eq!(active.deliveries.len(), 2);
        assert_eq!(active.deliveries[0].description, tr("AirPods Zubehör"));
        assert_eq!(recent.fetched_at_unix_secs, 1_800_000_100);
        assert_eq!(recent.deliveries.len(), 1);
        assert_eq!(recent.deliveries[0].description, tr("Monitorarm"));
    }

    #[test]
    fn replacing_a_snapshot_returns_the_previous_value() {
        let directory = TestDirectory::new("replace");
        let cache = DeliveryCache::open_in(&directory.0).expect("open cache");
        let deliveries = crate::model::demo_deliveries();

        assert!(
            cache
                .replace_snapshot(SnapshotKind::Active, 10, deliveries[..1].to_vec())
                .expect("first write")
                .is_none()
        );
        let previous = cache
            .replace_snapshot(SnapshotKind::Active, 20, deliveries[1..2].to_vec())
            .expect("second write")
            .expect("previous snapshot");

        assert_eq!(previous.fetched_at_unix_secs, 10);
        assert_eq!(previous.deliveries[0].description, tr("AirPods Zubehör"));
        assert_eq!(
            cache
                .snapshot(SnapshotKind::Active)
                .expect("read snapshot")
                .expect("current snapshot")
                .deliveries[0]
                .description,
            tr("Kaffeebohnen")
        );
    }

    #[test]
    fn clear_writes_an_empty_valid_envelope() {
        let directory = TestDirectory::new("clear");
        let cache = DeliveryCache::open_in(&directory.0).expect("open cache");
        cache
            .replace_snapshot(
                SnapshotKind::Active,
                10,
                crate::model::demo_deliveries()[..1].to_vec(),
            )
            .expect("write snapshot");

        cache.clear().expect("clear cache");
        drop(cache);

        let reopened = DeliveryCache::open_in(&directory.0).expect("reopen cache");
        assert!(
            reopened
                .snapshot(SnapshotKind::Active)
                .expect("read active")
                .is_none()
        );
        assert!(
            reopened
                .snapshot(SnapshotKind::Recent)
                .expect("read recent")
                .is_none()
        );
    }

    #[test]
    fn rejects_unknown_schema_versions() {
        let directory = TestDirectory::new("schema");
        ensure_private_directory(&directory.0).expect("create state directory");
        let path = directory.0.join(CACHE_FILE_NAME);
        fs::write(
            &path,
            br#"{"schema_version":99,"active":null,"recent":null}"#,
        )
        .expect("write future cache");

        let error = match DeliveryCache::open_in(&directory.0) {
            Ok(_) => panic!("future cache version must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StorageError::UnsupportedVersion {
                found: 99,
                expected: SCHEMA_VERSION
            }
        ));
    }

    #[test]
    fn rejects_truncated_json_instead_of_silently_losing_data() {
        let directory = TestDirectory::new("truncated");
        ensure_private_directory(&directory.0).expect("create state directory");
        fs::write(directory.0.join(CACHE_FILE_NAME), b"{\"schema_version\":1")
            .expect("write truncated cache");

        let error = match DeliveryCache::open_in(&directory.0) {
            Ok(_) => panic!("truncated cache must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, StorageError::InvalidJson(_)));
    }

    #[test]
    fn successful_writes_leave_no_temporary_files() {
        let directory = TestDirectory::new("temporary");
        let cache = DeliveryCache::open_in(&directory.0).expect("open cache");
        cache
            .replace_snapshot(
                SnapshotKind::Active,
                10,
                crate::model::demo_deliveries()[..1].to_vec(),
            )
            .expect("write snapshot");

        let names: Vec<_> = fs::read_dir(&directory.0)
            .expect("read state directory")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(CACHE_FILE_NAME)]);
    }

    #[cfg(unix)]
    #[test]
    fn state_directory_and_cache_file_are_private() {
        let directory = TestDirectory::new("permissions");
        let cache = DeliveryCache::open_in(&directory.0).expect("open cache");
        cache
            .replace_snapshot(SnapshotKind::Active, 10, Vec::new())
            .expect("write snapshot");

        let directory_mode = fs::metadata(&directory.0)
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(cache.path())
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_as_the_cache_file() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("symlink");
        ensure_private_directory(&directory.0).expect("create state directory");
        let unrelated = directory.0.join("unrelated.json");
        fs::write(&unrelated, b"do not touch").expect("write unrelated file");
        symlink(&unrelated, directory.0.join(CACHE_FILE_NAME)).expect("create symlink");

        let error = match DeliveryCache::open_in(&directory.0) {
            Ok(_) => panic!("cache symlink must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, StorageError::InvalidFile(_)));
        assert_eq!(
            fs::read(&unrelated).expect("read unrelated file"),
            b"do not touch"
        );
    }
}

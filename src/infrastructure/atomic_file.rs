//! Complete-file publication for serialized state. Callers own serialization and
//! cooperative writer guards; this helper does not promise power-loss durability.

// Override the parent module's legacy allowance for this new foundation.
#![deny(dead_code)]

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::application::ports::CleanupWarning;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    CreateNew,
    Replace,
}

#[derive(Debug)]
pub struct AtomicWriteOutcome {
    pub warnings: Vec<CleanupWarning>,
}

/// Resolve the physical regular-file destination without changing its logical
/// config-relative base. Missing files require an existing physical parent.
pub fn resolve_write_path(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let resolved = fs::canonicalize(path)?;
            if !fs::metadata(&resolved)?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "atomic write destination must be a regular file",
                ));
            }
            Ok(resolved)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "atomic write destination needs a file name",
                )
            })?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            Ok(fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

/// Publish complete bytes without truncating or unlinking the destination.
/// Every error is pre-publication; create-only post-publication cleanup problems
/// are warnings. Non-cooperating external editors are not isolated.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: WriteMode) -> io::Result<AtomicWriteOutcome> {
    write_atomic_with_hook(path, bytes, mode, |_phase, _| {
        #[cfg(test)]
        {
            TEST_FAULT.with(|fault| {
                if fault.get() == Some(_phase) {
                    return Err(io::Error::other("injected atomic write failure"));
                }
                Ok(())
            })?;
            if _phase == Phase::Publish {
                TEST_COMPETING_CREATE.with(|competitor| {
                    if let Some((target, content)) = competitor.borrow().as_ref() {
                        if target == path {
                            fs::write(target, content)?;
                        }
                    }
                    Ok::<_, io::Error>(())
                })?;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
thread_local! {
    static TEST_FAULT: std::cell::Cell<Option<Phase>> = const { std::cell::Cell::new(None) };
    static TEST_COMPETING_CREATE: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> = const { std::cell::RefCell::new(None) };
}

/// Thread-local, scoped injection: never affects parallel tests or production.
#[cfg(test)]
pub(crate) fn with_test_publication_failure<T>(run: impl FnOnce() -> T) -> T {
    with_test_failure(Phase::Publish, run)
}

#[cfg(test)]
pub(crate) fn with_test_cleanup_failure<T>(run: impl FnOnce() -> T) -> T {
    with_test_failure(Phase::Remove, run)
}

#[cfg(test)]
fn with_test_failure<T>(phase: Phase, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<Phase>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_FAULT.with(|fault| fault.set(self.0));
        }
    }
    let _reset = Reset(TEST_FAULT.with(|fault| fault.replace(Some(phase))));
    run()
}

#[cfg(test)]
pub(crate) fn with_test_competing_create<T>(
    path: PathBuf,
    bytes: Vec<u8>,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<(PathBuf, Vec<u8>)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_COMPETING_CREATE.with(|competitor| {
                competitor.replace(self.0.take());
            });
        }
    }
    let _reset =
        Reset(TEST_COMPETING_CREATE.with(|competitor| competitor.replace(Some((path, bytes)))));
    run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Write,
    Sync,
    Publish,
    Remove,
}

// Tests inject faults at byte-publication boundaries; production uses a no-op.
fn write_atomic_with_hook(
    path: &Path,
    bytes: &[u8],
    mode: WriteMode,
    hook: impl FnMut(Phase, &mut TemporaryFile) -> io::Result<()>,
) -> io::Result<AtomicWriteOutcome> {
    write_atomic_with_candidates(
        path,
        bytes,
        mode,
        |parent| {
            let parent = parent.to_path_buf();
            static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
            (0..128).map(move |_| {
                parent.join(format!(
                    ".sksync-write-{}-{}",
                    std::process::id(),
                    NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
                ))
            })
        },
        hook,
    )
}

// Deterministic candidate injection lets tests exercise final-path collisions
// without changing the process-wide counter or environment.
fn write_atomic_with_candidates<C: IntoIterator<Item = PathBuf>>(
    path: &Path,
    bytes: &[u8],
    mode: WriteMode,
    candidates: impl FnOnce(&Path) -> C,
    mut hook: impl FnMut(Phase, &mut TemporaryFile) -> io::Result<()>,
) -> io::Result<AtomicWriteOutcome> {
    let destination = resolve_write_path(path)?;
    let original = match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.is_file() => Some(metadata),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "atomic write destination must be a regular file",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if mode == WriteMode::CreateNew && original.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "atomic create destination already exists",
        ));
    }
    let parent = destination
        .parent()
        .expect("resolved destination has a parent");
    // Never open the final path as staging. Unicode filesystem aliases can
    // change prefix spelling, so exclude its ASCII sequence suffix independently
    // of prefix, as the sibling body allocator does. This is allocation-only;
    // arbitrary public output names remain valid.
    let candidates = candidates(parent).into_iter().filter(|candidate| {
        if candidate == &destination {
            return false;
        }
        let Some(name) = candidate.file_name() else {
            return true;
        };
        let bytes = name.as_encoded_bytes();
        let Some(separator) = bytes.iter().rposition(|byte| *byte == b'-') else {
            return true;
        };
        let suffix = &bytes[separator..];
        if suffix.len() == 1 || !suffix[1..].iter().all(u8::is_ascii_digit) {
            return true;
        }
        !destination
            .file_name()
            .is_some_and(|final_name| final_name.as_encoded_bytes().ends_with(suffix))
    });
    // Replacements are private from creation through writing. New destinations
    // retain normal creation permissions; original special bits are restored below.
    let creation_mode = if original.is_some() { 0o600 } else { 0o666 };
    let mut temporary = create_temporary(candidates, creation_mode)?;
    hook(Phase::Write, &mut temporary)?;
    temporary.file.write_all(bytes)?;
    if let Some(metadata) = &original {
        temporary.file.set_permissions(metadata.permissions())?;
    }
    hook(Phase::Sync, &mut temporary)?;
    temporary.file.sync_all()?;
    hook(Phase::Publish, &mut temporary)?;
    match mode {
        WriteMode::Replace => {
            // Do not replace a detected unexpected path. This is not isolation
            // against an external editor racing the following rename.
            match (original.as_ref(), fs::symlink_metadata(&destination)) {
                (Some(old), Ok(current)) if same_regular_file(old, &current) => {}
                (None, Err(error)) if error.kind() == io::ErrorKind::NotFound => {}
                (_, Err(error)) => return Err(error),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "atomic write destination changed before publication",
                    ))
                }
            }
            fs::rename(&temporary.path, &destination)?;
            temporary.cleanup_on_drop = false;
            Ok(AtomicWriteOutcome {
                warnings: Vec::new(),
            })
        }
        WriteMode::CreateNew => {
            // No overwriting-rename fallback if hard-link publication is unsupported.
            fs::hard_link(&temporary.path, &destination)?;
            let cleanup = hook(Phase::Remove, &mut temporary).and_then(|()| temporary.remove());
            // A failed cleanup must not be retried by Drop after commit.
            temporary.cleanup_on_drop = false;
            let warnings = cleanup
                .err()
                .map(|error| CleanupWarning {
                    path: temporary.path.clone(),
                    message: error.to_string(),
                })
                .into_iter()
                .collect();
            Ok(AtomicWriteOutcome { warnings })
        }
    }
}

fn same_regular_file(left: &Metadata, right: &Metadata) -> bool {
    left.is_file() && right.is_file() && left.dev() == right.dev() && left.ino() == right.ino()
}

struct TemporaryFile {
    path: PathBuf,
    file: File,
    cleanup_on_drop: bool,
}

impl TemporaryFile {
    fn remove(&mut self) -> io::Result<()> {
        if !same_regular_file(&self.file.metadata()?, &fs::symlink_metadata(&self.path)?) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "refusing to remove an unowned temporary path",
            ));
        }
        fs::remove_file(&self.path)?;
        self.cleanup_on_drop = false;
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = self.remove();
        }
    }
}

fn create_temporary(
    candidates: impl IntoIterator<Item = PathBuf>,
    creation_mode: u32,
) -> io::Result<TemporaryFile> {
    for path in candidates {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(creation_mode)
            .open(&path)
        {
            Ok(file) => {
                return Ok(TemporaryFile {
                    path,
                    file,
                    cleanup_on_drop: true,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not exclusively create an atomic write temporary file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{self, Write};
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::path::Path;

    #[test]
    fn atomic_create_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"old").unwrap();
        assert!(write_atomic(&path, b"new", WriteMode::CreateNew).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn atomic_replace_publishes_complete_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"old bytes longer than new").unwrap();
        let outcome = write_atomic(&path, b"new", WriteMode::Replace).unwrap();
        assert!(outcome.warnings.is_empty());
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_modes_create_absent_files_with_normal_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let ordinary = dir.path().join("ordinary");
        fs::write(&ordinary, b"ordinary").unwrap();
        for mode in [WriteMode::CreateNew, WriteMode::Replace] {
            let path = dir.path().join(format!("{mode:?}"));
            assert_eq!(
                resolve_write_path(&path).unwrap(),
                dir.path().canonicalize().unwrap().join(format!("{mode:?}"))
            );
            let outcome = write_atomic(&path, b"new", mode).unwrap();
            assert!(outcome.warnings.is_empty());
            assert_eq!(fs::read(&path).unwrap(), b"new");
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode(),
                fs::metadata(&ordinary).unwrap().permissions().mode()
            );
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn atomic_replace_preserves_symlink_chain_and_referent_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let physical_parent = dir.path().join("dotfiles");
        fs::create_dir(&physical_parent).unwrap();
        let referent = physical_parent.join("config.json");
        fs::write(&referent, b"old").unwrap();
        fs::set_permissions(&referent, fs::Permissions::from_mode(0o640)).unwrap();
        let alias = dir.path().join("alias");
        symlink("dotfiles/config.json", &alias).unwrap();
        let path = dir.path().join("config.json");
        symlink("alias", &path).unwrap();
        assert_eq!(
            resolve_write_path(&path).unwrap(),
            referent.canonicalize().unwrap()
        );
        write_atomic_with_hook(&path, b"new", WriteMode::Replace, |phase, temporary| {
            if phase == Phase::Sync {
                assert_eq!(
                    temporary.path.parent().unwrap(),
                    physical_parent.canonicalize().unwrap()
                );
                assert_eq!(fs::read(&referent).unwrap(), b"old");
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("alias"));
        assert_eq!(
            fs::read_link(&alias).unwrap(),
            Path::new("dotfiles/config.json")
        );
        assert_eq!(fs::read(&referent).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&referent).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert!(write_atomic(&path, b"other", WriteMode::CreateNew).is_err());
        assert_eq!(fs::read(&referent).unwrap(), b"new");
    }

    #[test]
    fn atomic_recovery_replacement_is_private_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        for bits in [0o600, 0o400, 0o4750] {
            let path = dir.path().join(format!("private-{bits:o}"));
            fs::write(&path, b"old private bytes").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(bits)).unwrap();
            let mut reached_write = false;
            let outcome = write_atomic_with_hook(
                &path,
                b"new private bytes",
                WriteMode::Replace,
                |phase, temporary| {
                    if phase == Phase::Write {
                        reached_write = true;
                        let mode = temporary.file.metadata()?.permissions().mode() & 0o7777;
                        assert_eq!(
                            mode & !0o600,
                            0,
                            "replacement temporary must be private before writing"
                        );
                        assert_eq!(temporary.file.metadata()?.len(), 0);
                        assert_eq!(fs::read(&path).unwrap(), b"old private bytes");
                    }
                    if phase == Phase::Sync {
                        assert_eq!(
                            temporary.file.metadata()?.permissions().mode() & 0o7777,
                            bits
                        );
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert!(reached_write);
            assert!(outcome.warnings.is_empty());
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
                bits
            );
            assert_eq!(fs::read(&path).unwrap(), b"new private bytes");
        }
    }

    #[test]
    fn atomic_recovery_partial_replacement_failure_stays_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.json");
        fs::write(&path, b"old private bytes").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut reached_write = false;
        let result = write_atomic_with_hook(
            &path,
            b"new private bytes",
            WriteMode::Replace,
            |phase, temporary| {
                if phase == Phase::Write {
                    reached_write = true;
                    temporary.file.write_all(b"partial private bytes")?;
                    assert_eq!(fs::read(&temporary.path).unwrap(), b"partial private bytes");
                    assert_eq!(
                        temporary.file.metadata()?.permissions().mode() & !0o600 & 0o7777,
                        0,
                        "partial replacement bytes must not be exposed"
                    );
                    return Err(io::Error::other("injected partial private write failure"));
                }
                Ok(())
            },
        );
        assert!(reached_write);
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old private bytes");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_recovery_destination_candidate_is_excluded() {
        for mode in [WriteMode::CreateNew, WriteMode::Replace] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let path = root.join(".sksync-write-destination");
            let next = root.join("temporary");
            let mut reached_publish = false;
            let outcome = write_atomic_with_candidates(
                &path,
                b"complete bytes",
                mode,
                |_| [path.clone(), next.clone()],
                |phase, temporary| {
                    if phase != Phase::Remove {
                        assert!(
                            !path.exists(),
                            "destination must stay absent before publication"
                        );
                        assert_eq!(temporary.path, next);
                    }
                    if phase == Phase::Publish {
                        reached_publish = true;
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert!(reached_publish);
            assert!(outcome.warnings.is_empty());
            assert_eq!(fs::read(&path).unwrap(), b"complete bytes");
            assert!(!next.exists());
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        }
    }

    fn assert_absent_alias_candidate_is_excluded(mode: WriteMode) {
        for final_name in [
            ".sKsync-write-123-0",
            ".ſksync-write-123-0",
            ".SKSYNC-WRITE-123-0",
            "unrelated-prefix-0",
        ] {
            for fail_at in [
                None,
                Some(Phase::Write),
                Some(Phase::Sync),
                Some(Phase::Publish),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path().canonicalize().unwrap();
                let path = root.join(final_name);
                let candidate = root.join(".sksync-write-123-0");
                let next = root.join(".sksync-write-123-1");
                // Probe actual temporary-filesystem alias behavior without leaving
                // the destination present when the atomic writer begins.
                fs::write(&candidate, b"alias probe").unwrap();
                eprintln!("{mode:?} {final_name}: aliases candidate={}", path.exists());
                fs::remove_file(&candidate).unwrap();
                let mut reached_publish = false;
                let result = write_atomic_with_candidates(
                    &path,
                    b"complete bytes",
                    mode,
                    |_| [candidate.clone(), next.clone()],
                    |phase, temporary| {
                        if phase != Phase::Remove {
                            assert!(
                                !path.exists(),
                                "{mode:?} {final_name}: destination appeared before {phase:?}"
                            );
                            assert_eq!(temporary.path, next, "matching ASCII sequence suffix must be excluded independently of prefix");
                        }
                        if phase == Phase::Publish {
                            reached_publish = true;
                        }
                        if Some(phase) == fail_at {
                            return Err(io::Error::other("injected alias publication failure"));
                        }
                        Ok(())
                    },
                );
                if fail_at.is_some() {
                    assert!(result.is_err());
                    assert!(!path.exists());
                    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
                } else {
                    assert!(result.unwrap().warnings.is_empty());
                    assert!(reached_publish);
                    assert_eq!(fs::read(&path).unwrap(), b"complete bytes");
                    assert!(!next.exists());
                    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
                }
            }
        }
    }

    #[test]
    fn atomic_create_excludes_alias_candidate_before_publication() {
        assert_absent_alias_candidate_is_excluded(WriteMode::CreateNew);
    }

    #[test]
    fn atomic_absent_replace_excludes_alias_candidate_before_publication() {
        assert_absent_alias_candidate_is_excluded(WriteMode::Replace);
    }

    #[test]
    fn atomic_alias_candidate_preserves_existing_file_and_private_replacement() {
        for final_name in [".sKsync-write-123-0", ".ſksync-write-123-0"] {
            for fail_at in [
                None,
                Some(Phase::Write),
                Some(Phase::Sync),
                Some(Phase::Publish),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path().canonicalize().unwrap();
                let path = root.join(final_name);
                let candidate = root.join(".sksync-write-123-0");
                let next = root.join(".sksync-write-123-1");
                fs::write(&path, b"old private bytes").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                assert!(write_atomic_with_candidates(
                    &path,
                    b"new",
                    WriteMode::CreateNew,
                    |_| [candidate.clone(), next.clone()],
                    |_, _| panic!("existing create must not stage")
                )
                .is_err());
                assert_eq!(fs::read(&path).unwrap(), b"old private bytes");
                let result = write_atomic_with_candidates(
                    &path,
                    b"new",
                    WriteMode::Replace,
                    |_| [candidate.clone(), next.clone()],
                    |phase, temporary| {
                        assert_eq!(temporary.path, next);
                        assert_eq!(fs::read(&path).unwrap(), b"old private bytes");
                        assert_eq!(
                            temporary.file.metadata()?.permissions().mode() & 0o7777,
                            0o600
                        );
                        if Some(phase) == fail_at {
                            return Err(io::Error::other("injected private alias write failure"));
                        }
                        Ok(())
                    },
                );
                if fail_at.is_some() {
                    assert!(result.is_err());
                    assert_eq!(fs::read(&path).unwrap(), b"old private bytes");
                } else {
                    assert!(result.unwrap().warnings.is_empty());
                    assert_eq!(fs::read(&path).unwrap(), b"new");
                }
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
                    0o600
                );
                assert!(!next.exists());
                assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
            }
        }
    }

    #[test]
    fn atomic_replace_preserves_unix_permissions_including_readonly() {
        let dir = tempfile::tempdir().unwrap();
        for bits in [0o440, 0o640, 0o750, 0o4750] {
            let path = dir.path().join(format!("{bits:o}"));
            fs::write(&path, b"old").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(bits)).unwrap();
            write_atomic(&path, b"new", WriteMode::Replace).unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
                bits
            );
            assert_eq!(fs::read(&path).unwrap(), b"new");
        }
    }

    #[test]
    fn atomic_rejects_dangling_links_and_non_regular_referents() {
        let dir = tempfile::tempdir().unwrap();
        let dangling = dir.path().join("dangling");
        symlink("missing", &dangling).unwrap();
        let directory = dir.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let alias = dir.path().join("directory-alias");
        symlink(&directory, &alias).unwrap();
        let socket = dir.path().join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        for path in [&dangling, &directory, &alias, &socket] {
            assert!(resolve_write_path(path).is_err());
            for mode in [WriteMode::CreateNew, WriteMode::Replace] {
                assert!(write_atomic(path, b"new", mode).is_err());
                assert!(fs::symlink_metadata(path).is_ok());
            }
        }
        assert!(!dir.path().join("missing").exists());
        assert_eq!(fs::read_link(&dangling).unwrap(), Path::new("missing"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }

    #[test]
    fn atomic_missing_parent_fails_without_creating_containers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/config.json");
        assert!(write_atomic(&path, b"new", WriteMode::Replace).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn atomic_prepublication_failures_preserve_bytes_or_absence() {
        for phase in [Phase::Write, Phase::Sync, Phase::Publish] {
            for exists in [false, true] {
                for mode in [WriteMode::CreateNew, WriteMode::Replace] {
                    if exists && mode == WriteMode::CreateNew {
                        continue;
                    }
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("config.json");
                    if exists {
                        fs::write(&path, b"old raw bytes\n").unwrap();
                    }
                    let mut reached = false;
                    let result = write_atomic_with_hook(&path, b"new", mode, |at, temporary| {
                        if at == phase {
                            reached = true;
                            if at == Phase::Write {
                                temporary.file.write_all(b"partial")?;
                            }
                            return Err(io::Error::other("injected failure"));
                        }
                        Ok(())
                    });
                    assert!(reached);
                    assert!(result.is_err());
                    if exists {
                        assert_eq!(fs::read(&path).unwrap(), b"old raw bytes\n");
                    } else {
                        assert!(!path.exists());
                    }
                    assert_eq!(
                        fs::read_dir(dir.path()).unwrap().count(),
                        usize::from(exists)
                    );
                }
            }
        }
    }

    #[test]
    fn atomic_create_rejects_competing_publication_without_removing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let result = write_atomic_with_hook(&path, b"new", WriteMode::CreateNew, |phase, _| {
            if phase == Phase::Publish {
                fs::write(&path, b"competitor")?;
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"competitor");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_create_cleanup_failure_is_a_committed_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let outcome = write_atomic_with_hook(&path, b"new", WriteMode::CreateNew, |phase, _| {
            if phase == Phase::Remove {
                return Err(io::Error::other("injected removal failure"));
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(outcome.warnings.len(), 1);
        let warning = &outcome.warnings[0];
        assert!(warning.message.contains("injected removal failure"));
        assert_eq!(fs::read(&warning.path).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&path).unwrap().ino(),
            fs::metadata(&warning.path).unwrap().ino()
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn atomic_replace_has_no_postpublication_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"old").unwrap();
        let outcome = write_atomic_with_hook(&path, b"new", WriteMode::Replace, |phase, _| {
            assert_ne!(phase, Phase::Remove);
            Ok(())
        })
        .unwrap();
        assert!(outcome.warnings.is_empty());
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn atomic_temporary_collisions_are_never_overwritten_or_followed() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join("foreign");
        fs::write(&foreign, b"foreign").unwrap();
        let alias = dir.path().join("alias");
        symlink(&foreign, &alias).unwrap();
        let directory = dir.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let owned = dir.path().join("owned");
        let temporary = create_temporary(
            [
                foreign.clone(),
                alias.clone(),
                directory.clone(),
                owned.clone(),
            ],
            0o666,
        )
        .unwrap();
        assert_eq!(temporary.path, owned);
        drop(temporary);
        assert!(!owned.exists());
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read_link(&alias).unwrap(), foreign);
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn atomic_exhausted_temporary_collisions_leave_foreign_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join("foreign");
        fs::write(&foreign, b"foreign").unwrap();
        assert!(create_temporary([foreign.clone()], 0o666).is_err());
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_replace_rejects_a_detected_destination_change() {
        for exists in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let foreign = dir.path().join("foreign");
            fs::write(&foreign, b"foreign").unwrap();
            if exists {
                fs::write(&path, b"old").unwrap();
            }
            let result = write_atomic_with_hook(&path, b"new", WriteMode::Replace, |phase, _| {
                if phase == Phase::Publish {
                    if exists {
                        fs::rename(&path, dir.path().join("retained-old"))?;
                    }
                    symlink(&foreign, &path)?;
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(fs::read_link(&path).unwrap(), foreign);
            assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
            if exists {
                assert_eq!(fs::read(dir.path().join("retained-old")).unwrap(), b"old");
            }
            assert_eq!(
                fs::read_dir(dir.path()).unwrap().count(),
                2 + usize::from(exists)
            );
        }
    }

    #[test]
    fn atomic_missing_file_uses_canonical_parent_alias() {
        let dir = tempfile::tempdir().unwrap();
        let physical = dir.path().join("physical");
        fs::create_dir(&physical).unwrap();
        let alias = dir.path().join("alias");
        symlink(&physical, &alias).unwrap();
        let path = alias.join("config.json");
        assert_eq!(
            resolve_write_path(&path).unwrap(),
            physical.canonicalize().unwrap().join("config.json")
        );
        write_atomic(&path, b"new", WriteMode::CreateNew).unwrap();
        assert_eq!(fs::read(physical.join("config.json")).unwrap(), b"new");
        assert_eq!(fs::read_link(&alias).unwrap(), physical);
    }

    #[test]
    fn atomic_parent_permission_failure_preserves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        // Root bypasses Unix directory permissions; do not mutate process identity.
        if fs::metadata(dir.path()).unwrap().uid() == 0 {
            return;
        }
        let path = dir.path().join("config.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = write_atomic(&path, b"new", WriteMode::Replace);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_cleanup_does_not_remove_a_foreign_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("temporary");
        let mut temporary = create_temporary([path.clone()], 0o666).unwrap();
        let foreign = dir.path().join("foreign");
        fs::write(&foreign, b"foreign").unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&foreign, &path).unwrap();
        assert!(temporary.remove().is_err());
        drop(temporary);
        assert_eq!(fs::read_link(&path).unwrap(), foreign);
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");

        fs::remove_file(&path).unwrap();
        let mut temporary = create_temporary([path.clone()], 0o666).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"foreign replacement").unwrap();
        assert!(temporary.remove().is_err());
        drop(temporary);
        assert_eq!(fs::read(&path).unwrap(), b"foreign replacement");
    }
}

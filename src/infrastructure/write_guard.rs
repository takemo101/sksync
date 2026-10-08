//! Cooperative, non-blocking writer exclusion on Unix directory handles.
//!
//! Lock directories, not replaceable state-file inodes. Drop unlocks then closes
//! the handles, including on acquisition errors. The OS releases locks on process
//! termination.
//! These advisory guards do not isolate readers or non-cooperating editors.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

// P06 consumes this foundation API at mutation boundaries; no command entry
// point acquires guards until that issue.
pub struct WriteGuard {
    directories: Vec<PathBuf>,
    handles: Vec<File>,
}

impl WriteGuard {
    pub fn acquire(directories: &[PathBuf]) -> io::Result<Self> {
        let mut directories = directories
            .iter()
            .map(fs::canonicalize)
            .collect::<io::Result<Vec<_>>>()?;
        directories.sort();
        directories.dedup();

        let mut guard = Self {
            directories: Vec::new(),
            handles: Vec::new(),
        };
        let mut identities = BTreeSet::new();
        for directory in directories {
            // Opening a FIFO for reading can block before handle validation.
            // Reject non-directories first, then still validate the opened handle.
            if !fs::metadata(&directory)?.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Writer guards require existing directories",
                ));
            }
            let handle = File::open(&directory)?;
            let metadata = handle.metadata()?;
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Writer guards require existing directories",
                ));
            }
            if !identities.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            handle.try_lock().map_err(io::Error::from)?;
            guard.directories.push(directory);
            guard.handles.push(handle);
        }
        Ok(guard)
    }

    pub fn covers(&self, directories: &[PathBuf]) -> io::Result<bool> {
        for directory in directories {
            let canonical = fs::canonicalize(directory)?;
            let current = fs::metadata(&canonical)?;
            if !current.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Writer guards require existing directories",
                ));
            }
            let identity = (current.dev(), current.ino());
            match self.directories.binary_search(&canonical) {
                Ok(index) => {
                    let held = self.handles[index].metadata()?;
                    if identity != (held.dev(), held.ino()) {
                        return Ok(false);
                    }
                }
                Err(_) => {
                    // Identity deduplication can omit a distinct canonical alias
                    // (e.g. a bind mount). It is covered by the same held inode.
                    let mut covered = false;
                    for handle in &self.handles {
                        let held = handle.metadata()?;
                        if identity == (held.dev(), held.ino()) {
                            covered = true;
                            break;
                        }
                    }
                    if !covered {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        for handle in &self.handles {
            // A concurrent fork can inherit the open-file description before
            // exec. Explicit unlock releases it immediately even in that window.
            // Never replace an acquisition error or panic during cleanup; closing
            // the private handles remains the fallback if unlock fails.
            let _ = handle.unlock();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_guard_covers_aliases_but_rejects_replaced_directories() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("store");
        let alias = root.path().join("alias");
        fs::create_dir(&directory).unwrap();
        std::os::unix::fs::symlink(&directory, &alias).unwrap();
        let guard = WriteGuard::acquire(std::slice::from_ref(&directory)).unwrap();
        assert!(guard.covers(std::slice::from_ref(&alias)).unwrap());
        assert!(!guard.covers(&[root.path().to_path_buf()]).unwrap());

        fs::rename(&directory, root.path().join("old-store")).unwrap();
        fs::create_dir(&directory).unwrap();
        assert!(!guard.covers(std::slice::from_ref(&directory)).unwrap());
        assert!(!guard.covers(std::slice::from_ref(&alias)).unwrap());
        assert!(WriteGuard::acquire(&[directory]).is_ok());
    }

    #[test]
    fn write_guard_covers_distinct_canonical_paths_with_same_identity() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("Store");
        let alias = root.path().join("store");
        fs::create_dir(&directory).unwrap();
        #[cfg(target_os = "macos")]
        let alias = {
            // APFS firmlinks expose Data-volume directories under distinct
            // canonical paths without requiring a privileged bind mount.
            let canonical = fs::canonicalize(&directory).unwrap();
            let data_alias =
                PathBuf::from("/System/Volumes/Data").join(canonical.strip_prefix("/").unwrap());
            if data_alias.exists() && fs::canonicalize(&data_alias).unwrap() != canonical {
                data_alias
            } else {
                alias
            }
        };
        if !alias.exists() {
            // Case-sensitive filesystems have no case aliases. Verify that a
            // distinct physical resource is not mistaken for a covered alias.
            fs::create_dir(&alias).unwrap();
            let guard = WriteGuard::acquire(&[directory]).unwrap();
            assert!(!guard.covers(&[alias]).unwrap());
            return;
        }
        let canonical = fs::canonicalize(&directory).unwrap();
        let canonical_alias = fs::canonicalize(&alias).unwrap();
        if canonical == canonical_alias {
            // Some filesystems normalize spelling during canonicalization.
            return;
        }
        let original = fs::metadata(&canonical).unwrap();
        let aliased = fs::metadata(&canonical_alias).unwrap();
        assert_eq!(
            (original.dev(), original.ino()),
            (aliased.dev(), aliased.ino())
        );
        let resources = vec![directory.clone(), alias.clone()];
        let guard = WriteGuard::acquire(&resources).unwrap();
        assert_eq!(guard.handles.len(), 1);
        assert!(guard.covers(&resources).unwrap());
        assert!(guard.covers(std::slice::from_ref(&alias)).unwrap());
        fs::rename(&directory, root.path().join("old-store")).unwrap();
        fs::create_dir(&directory).unwrap();
        assert!(!guard.covers(&resources).unwrap());
        assert!(!guard.covers(&[alias]).unwrap());
    }

    #[test]
    fn write_guard_rejects_swapped_inodes_at_retained_paths() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        let moving = root.path().join("moving");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let resources = vec![first.clone(), second.clone()];
        let guard = WriteGuard::acquire(&resources).unwrap();
        fs::rename(&first, &moving).unwrap();
        fs::rename(&second, &first).unwrap();
        fs::rename(&moving, &second).unwrap();
        assert!(!guard.covers(&resources).unwrap());
    }

    #[test]
    fn write_guard_deduplicates_aliases_and_acquires_in_sorted_order() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a-store");
        let second = root.path().join("z-store");
        let alias = root.path().join("alias");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let requests = vec![second.clone(), alias, first.clone(), second.clone()];
        let guard = WriteGuard::acquire(&requests).unwrap();
        assert!(guard.covers(&requests).unwrap());
        assert_eq!(
            guard.directories,
            vec![
                fs::canonicalize(&first).unwrap(),
                fs::canonicalize(&second).unwrap()
            ]
        );
        assert_eq!(guard.handles.len(), 2);
        for directory in [&first, &second] {
            let error = WriteGuard::acquire(std::slice::from_ref(directory))
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        }
        drop(guard);
        assert!(WriteGuard::acquire(&requests).is_ok());
    }

    #[test]
    fn write_guard_releases_earlier_handles_after_contention() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a-store");
        let second = root.path().join("z-store");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let busy = WriteGuard::acquire(std::slice::from_ref(&second)).unwrap();
        let error = WriteGuard::acquire(&[second.clone(), first.clone()])
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(WriteGuard::acquire(std::slice::from_ref(&first)).is_ok());
        drop(busy);
        assert!(WriteGuard::acquire(&[first, second]).is_ok());
    }

    #[test]
    fn write_guard_rejects_non_directories_and_missing_paths_without_side_effects() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a-store");
        let file = root.path().join("z-file");
        let missing = root.path().join("missing");
        let dangling = root.path().join("dangling");
        fs::create_dir(&first).unwrap();
        fs::write(&file, b"unmanaged").unwrap();
        std::os::unix::fs::symlink(&missing, &dangling).unwrap();
        let error = WriteGuard::acquire(&[first.clone(), file.clone()])
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let guard = WriteGuard::acquire(&[first]).unwrap();
        assert_eq!(
            guard.covers(&[file.clone()]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        for path in [&missing, &dangling] {
            assert_eq!(
                WriteGuard::acquire(std::slice::from_ref(path))
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::NotFound
            );
            assert_eq!(
                guard.covers(std::slice::from_ref(path)).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
        }
        assert_eq!(fs::read(file).unwrap(), b"unmanaged");
        assert!(!missing.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }

    #[test]
    fn write_guard_rejects_fifos_without_blocking_and_releases_earlier_handles() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("a-store");
        let fifo = root.path().join("z-fifo");
        let alias = root.path().join("fifo-link");
        let ready = root.path().join("ready");
        let home = root.path().join("home");
        let xdg = home.join(".config");
        fs::create_dir(&directory).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        let output = Command::new("mkfifo")
            .arg(&fifo)
            .current_dir(root.path())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &xdg)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        std::os::unix::fs::symlink(&fifo, &alias).unwrap();

        let mut failures = Vec::new();
        for resource in [&fifo, &alias] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infrastructure::write_guard::tests::write_guard_fifo_child",
                    "--nocapture",
                ])
                .current_dir(root.path())
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("XDG_CONFIG_HOME", &xdg)
                .env("SKSYNC_WRITE_GUARD_TEST_DIRECTORY", &directory)
                .env("SKSYNC_WRITE_GUARD_FIFO_RESOURCE", resource)
                .env("SKSYNC_WRITE_GUARD_FIFO_READY", &ready)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let timed_out = loop {
                if child.try_wait().unwrap().is_some() {
                    break false;
                }
                if Instant::now() >= deadline {
                    break true;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if timed_out {
                let earlier_busy = WriteGuard::acquire(std::slice::from_ref(&directory))
                    .err()
                    .map(|error| error.kind());
                failures.push(format!(
                    "{resource:?}: rejection exceeded 5 seconds; child ready={}, earlier lock={earlier_busy:?}",
                    ready.exists()
                ));
                child.kill().unwrap();
            }
            let output = child.wait_with_output().unwrap();
            if !timed_out && !output.status.success() {
                failures.push(format!("{resource:?}: {output:?}"));
            }
            assert!(WriteGuard::acquire(std::slice::from_ref(&directory)).is_ok());
            fs::remove_file(&ready).unwrap();
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn write_guard_fifo_child() {
        let Some(resource) = std::env::var_os("SKSYNC_WRITE_GUARD_FIFO_RESOURCE") else {
            return;
        };
        let directory =
            PathBuf::from(std::env::var_os("SKSYNC_WRITE_GUARD_TEST_DIRECTORY").unwrap());
        let ready = PathBuf::from(std::env::var_os("SKSYNC_WRITE_GUARD_FIFO_READY").unwrap());
        fs::write(ready, b"ready").unwrap();
        let error = WriteGuard::acquire(&[directory.clone(), PathBuf::from(resource)])
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // Reacquire in this process, before exit can mask an error-path leak.
        assert!(WriteGuard::acquire(&[directory]).is_ok());
    }

    #[test]
    fn write_guard_drop_releases_lock_with_inherited_description_alive() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().to_path_buf();
        let guard = WriteGuard::acquire(std::slice::from_ref(&directory)).unwrap();
        // Model a fork retaining the open-file description before exec closes it.
        let inherited = guard.handles[0].try_clone().unwrap();
        drop(guard);
        assert!(WriteGuard::acquire(&[directory]).is_ok());
        drop(inherited);
    }

    #[test]
    fn write_guard_empty_resources_are_a_no_op() {
        let guard = WriteGuard::acquire(&[]).unwrap();
        assert!(guard.covers(&[]).unwrap());
    }

    #[test]
    fn write_guard_cross_process_exclusion_and_exit_release() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("store");
        let home = root.path().join("home");
        let xdg = home.join(".config");
        fs::create_dir(&directory).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        let run_child = |mode| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infrastructure::write_guard::tests::write_guard_child",
                    "--nocapture",
                ])
                .current_dir(root.path())
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("XDG_CONFIG_HOME", &xdg)
                .env("SKSYNC_WRITE_GUARD_TEST_DIRECTORY", &directory)
                .env("SKSYNC_WRITE_GUARD_TEST_MODE", mode)
                .output()
                .unwrap()
        };
        let guard = WriteGuard::acquire(std::slice::from_ref(&directory)).unwrap();
        let blocked = run_child("blocked");
        assert!(blocked.status.success(), "{blocked:?}");
        drop(guard);
        let exiting = run_child("exit");
        assert!(exiting.status.success(), "{exiting:?}");
        assert!(WriteGuard::acquire(&[directory]).is_ok());
    }

    #[test]
    fn write_guard_child() {
        let Some(directory) = std::env::var_os("SKSYNC_WRITE_GUARD_TEST_DIRECTORY") else {
            return;
        };
        let result = WriteGuard::acquire(&[PathBuf::from(directory)]);
        match std::env::var("SKSYNC_WRITE_GUARD_TEST_MODE")
            .unwrap()
            .as_str()
        {
            "blocked" => assert_eq!(result.err().unwrap().kind(), io::ErrorKind::WouldBlock),
            "exit" => {
                let _guard = result.unwrap();
                // Exit without Rust destructors: the OS must release the handle locks.
                std::process::exit(0);
            }
            mode => panic!("Unexpected writer guard child mode: {mode}"),
        }
    }

    #[test]
    fn write_guard_rejects_second_writer() {
        let dir = tempfile::tempdir().unwrap();
        let first = WriteGuard::acquire(&[dir.path().to_path_buf()]).unwrap();
        let error = WriteGuard::acquire(&[dir.path().to_path_buf()])
            .err()
            .unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        drop(first);
        assert!(WriteGuard::acquire(&[dir.path().to_path_buf()]).is_ok());
    }
}

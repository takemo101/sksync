use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest as ShaDigest, Sha256};
use thiserror::Error;
use walkdir::{DirEntry, WalkDir};

use crate::application::ports::{SourceHash, SourceHashStore, SourceHashStoreError};
use crate::domain::lockfile::Digest;
use crate::domain::skill::SourcePath;

#[derive(Debug, Error)]
pub enum HashError {
    #[error("failed to walk source directory {path}: {source}")]
    Walk {
        path: String,
        #[source]
        source: walkdir::Error,
    },
    #[error("failed to read file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to build digest: {0}")]
    Digest(#[from] crate::domain::lockfile::DigestError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryHash {
    pub hash: Digest,
    pub files: Vec<FileHash>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHash {
    pub path: PathBuf,
    pub hash: Digest,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Sha256SourceHashStore;

impl SourceHashStore for Sha256SourceHashStore {
    fn hash_source(&self, source: &SourcePath) -> Result<SourceHash, SourceHashStoreError> {
        hash_directory(source.as_path())
            .map(|directory| SourceHash {
                hash: directory.hash,
            })
            .map_err(|error| SourceHashStoreError::Hash {
                path: source.as_path().display().to_string(),
                message: error.to_string(),
            })
    }
}

pub fn hash_directory(source_dir: impl AsRef<Path>) -> Result<DirectoryHash, HashError> {
    let source_dir = source_dir.as_ref();
    let mut files = collect_files(source_dir)?;
    files.sort();

    let mut file_hashes = Vec::with_capacity(files.len());
    for relative_path in files {
        let absolute_path = source_dir.join(&relative_path);
        let bytes = fs::read(&absolute_path).map_err(|source| HashError::Read {
            path: absolute_path.display().to_string(),
            source,
        })?;
        let hash = Digest::new(format!("sha256-{}", hex::encode(Sha256::digest(&bytes))))?;
        file_hashes.push(FileHash {
            path: relative_path,
            hash,
        });
    }

    let mut directory_hasher = Sha256::new();
    for file in &file_hashes {
        directory_hasher.update(file.path.to_string_lossy().as_bytes());
        directory_hasher.update([0]);
        directory_hasher.update(file.hash.as_str().as_bytes());
        directory_hasher.update([0]);
    }

    Ok(DirectoryHash {
        hash: Digest::new(format!(
            "sha256-{}",
            hex::encode(directory_hasher.finalize())
        ))?,
        files: file_hashes,
    })
}

fn collect_files(source_dir: &Path) -> Result<Vec<PathBuf>, HashError> {
    let mut files = Vec::new();
    for entry in WalkDir::new(source_dir)
        .into_iter()
        .filter_entry(should_descend_into)
    {
        let entry = entry.map_err(|source| HashError::Walk {
            path: source_dir.display().to_string(),
            source,
        })?;
        if entry.file_type().is_file() {
            let relative_path = entry
                .path()
                .strip_prefix(source_dir)
                .expect("walkdir entry should be under source dir")
                .to_path_buf();
            files.push(relative_path);
        }
    }

    Ok(files)
}

fn should_descend_into(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }

    let name = entry.file_name().to_string_lossy();
    !matches!(name.as_ref(), ".git" | "target" | "node_modules")
}

#[cfg(test)]
mod tests {
    use super::hash_directory;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn same_contents_produce_same_hash() {
        let left = tempfile::tempdir().expect("left temp dir");
        let right = tempfile::tempdir().expect("right temp dir");
        fs::write(left.path().join("SKILL.md"), "hello").expect("write left skill");
        fs::write(right.path().join("SKILL.md"), "hello").expect("write right skill");

        let left_hash = hash_directory(left.path()).expect("hash left");
        let right_hash = hash_directory(right.path()).expect("hash right");

        assert_eq!(left_hash, right_hash);
    }

    #[test]
    fn content_changes_update_directory_hash() {
        let dir = tempfile::tempdir().expect("temp dir");
        let skill = dir.path().join("SKILL.md");
        fs::write(&skill, "hello").expect("write skill");
        let before = hash_directory(dir.path()).expect("hash before");

        fs::write(&skill, "hello world").expect("update skill");
        let after = hash_directory(dir.path()).expect("hash after");

        assert_ne!(before.hash, after.hash);
        assert_ne!(before.files[0].hash, after.files[0].hash);
    }

    #[test]
    fn file_creation_order_does_not_affect_hash() {
        let left = tempfile::tempdir().expect("left temp dir");
        let right = tempfile::tempdir().expect("right temp dir");
        fs::write(left.path().join("a.txt"), "A").expect("write a");
        fs::write(left.path().join("b.txt"), "B").expect("write b");
        fs::write(right.path().join("b.txt"), "B").expect("write b");
        fs::write(right.path().join("a.txt"), "A").expect("write a");

        let left_hash = hash_directory(left.path()).expect("hash left");
        let right_hash = hash_directory(right.path()).expect("hash right");

        assert_eq!(left_hash, right_hash);
        assert_eq!(
            left_hash
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]
        );
    }

    #[test]
    fn excluded_directories_do_not_affect_hash() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("SKILL.md"), "hello").expect("write skill");
        let before = hash_directory(dir.path()).expect("hash before");

        fs::create_dir(dir.path().join(".git")).expect("create git dir");
        fs::write(dir.path().join(".git/HEAD"), "ignored").expect("write ignored file");
        fs::create_dir(dir.path().join("target")).expect("create target dir");
        fs::write(dir.path().join("target/output"), "ignored").expect("write ignored file");
        fs::create_dir(dir.path().join("node_modules")).expect("create node_modules dir");
        fs::write(dir.path().join("node_modules/output"), "ignored").expect("write ignored file");
        let after = hash_directory(dir.path()).expect("hash after");

        assert_eq!(before, after);
    }

    #[test]
    fn excluded_directory_names_are_allowed_as_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("target"), "tracked").expect("write target file");

        let hash = hash_directory(dir.path()).expect("hash dir");

        assert_eq!(hash.files.len(), 1);
        assert_eq!(hash.files[0].path, PathBuf::from("target"));
    }
}

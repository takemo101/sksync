use std::path::Path;
use std::process::Command;

use thiserror::Error;

use crate::domain::source::GitInstallSource;

#[derive(Debug, Clone, Default)]
pub struct GitClient;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("git command failed for {repo}: {message}")]
pub struct GitCommandError {
    pub repo: String,
    pub message: String,
}

impl GitClient {
    pub fn clone_checkout(
        &self,
        source: &GitInstallSource,
        clone_dir: &Path,
    ) -> Result<(), GitCommandError> {
        self.clone_no_checkout(&source.url, clone_dir)?;
        self.checkout_reference(clone_dir, &source.url, source.wanted_ref())
    }

    pub fn resolve_head(&self, clone_dir: &Path, repo: &str) -> Result<String, GitCommandError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(clone_dir)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .map_err(|error| GitCommandError {
                repo: repo.to_owned(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(GitCommandError {
                repo: repo.to_owned(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn clone_no_checkout(&self, repo: &str, clone_dir: &Path) -> Result<(), GitCommandError> {
        let output = Command::new("git")
            .arg("clone")
            .arg("--filter=blob:none")
            .arg("--no-checkout")
            .arg(repo)
            .arg(clone_dir)
            .output()
            .map_err(|error| GitCommandError {
                repo: repo.to_owned(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(GitCommandError {
                repo: repo.to_owned(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    fn checkout_reference(
        &self,
        clone_dir: &Path,
        repo: &str,
        reference: &str,
    ) -> Result<(), GitCommandError> {
        if self
            .run_git(clone_dir, repo, &["checkout", "--detach", reference])
            .is_ok()
        {
            return Ok(());
        }

        self.run_git(
            clone_dir,
            repo,
            &["fetch", "--depth", "1", "origin", reference],
        )?;
        self.run_git(clone_dir, repo, &["checkout", "--detach", "FETCH_HEAD"])
    }

    fn run_git(&self, clone_dir: &Path, repo: &str, args: &[&str]) -> Result<(), GitCommandError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(clone_dir)
            .args(args)
            .output()
            .map_err(|error| GitCommandError {
                repo: repo.to_owned(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(GitCommandError {
                repo: repo.to_owned(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }
}

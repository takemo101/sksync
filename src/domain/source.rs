use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource {
    Git(GitInstallSource),
    Local(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInstallSource {
    pub url: String,
    pub reference: Option<String>,
    pub path: PathBuf,
}

impl GitInstallSource {
    pub fn wanted_ref(&self) -> &str {
        self.reference.as_deref().unwrap_or("HEAD")
    }
}

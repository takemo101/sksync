use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::application::config::ResolvedConfig;
use crate::application::ports::ConfigStore;
use crate::infrastructure::json::FileConfigStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigScope {
    Project,
    Global,
}

impl ConfigScope {
    pub(super) fn is_global(self) -> bool {
        matches!(self, Self::Global)
    }
}

impl fmt::Display for ConfigScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Project => "project config (./sksync.config.json)",
            Self::Global => "global config (~/.sksync/config.json)",
        };
        formatter.write_str(label)
    }
}

pub(super) fn load_config_for_scope(
    project_root: &Path,
    scope: ConfigScope,
) -> Result<ResolvedConfig> {
    let path = config_path_for_scope(project_root, scope)?;
    if !path.exists() {
        bail!("config not found: {}", path.display());
    }
    FileConfigStore::new(path)
        .load()
        .context("failed to load config")
}

pub(super) fn load_optional_config_for_scope(
    project_root: &Path,
    scope: ConfigScope,
) -> Result<Option<ResolvedConfig>> {
    let path = config_path_for_scope(project_root, scope)?;
    if !path.exists() {
        return Ok(None);
    }
    FileConfigStore::new(path)
        .load()
        .map(Some)
        .context("failed to load config")
}

pub(super) fn config_path_for_scope(project_root: &Path, scope: ConfigScope) -> Result<PathBuf> {
    match scope {
        ConfigScope::Project => Ok(project_root.join("sksync.config.json")),
        ConfigScope::Global => Ok(global_config_root()?.join("config.json")),
    }
}

pub(super) fn global_config_root() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|dir| dir.join(".sksync"))
        .context("failed to determine home directory for global sksync directory")
}

#[cfg(test)]
mod tests {
    use super::{config_path_for_scope, ConfigScope};
    use std::path::Path;

    #[test]
    fn project_scope_uses_project_config_path() {
        assert_eq!(
            config_path_for_scope(Path::new("/tmp/project"), ConfigScope::Project).unwrap(),
            Path::new("/tmp/project/sksync.config.json")
        );
    }
}

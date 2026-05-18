use std::path::Path;

use thiserror::Error;

use super::config::{ConfigResolveError, ResolvedConfig};

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Resolve(#[from] ConfigResolveError),
}

pub trait ConfigStore {
    fn load(&self) -> Result<ResolvedConfig, ConfigStoreError>;
}

pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}

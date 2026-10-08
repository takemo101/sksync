use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::config::{config_path_for_scope, load_optional_config_for_scope, ConfigScope};
use super::{prompt_config_scope, prompt_default_agents};
use crate::application::config::ResolvedConfig;
use crate::infrastructure::atomic_file::{write_atomic, WriteMode};

pub(super) fn run(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should store default agents?")?;
    let config = load_optional_config_for_scope(project_root, scope)?;
    let current_defaults = default_agents_from_config(config.as_ref());
    let agents = prompt_default_agents(scope, config.as_ref(), &current_defaults)?;
    let config_path = config_path_for_scope(project_root, scope)?;
    write_default_agents_config(&config_path, default_skill_dir_for_scope(scope), &agents)?;
    println!("✓ Updated default agents in {}", config_path.display());
    Ok(())
}

pub(super) fn default_agents_from_config(config: Option<&ResolvedConfig>) -> Vec<String> {
    config
        .map(|config| {
            config
                .default_agents
                .iter()
                .map(|agent| agent.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn write_default_agents_config(
    config_path: &Path,
    default_skill_dir: &str,
    agents: &[String],
) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut value = if config_path.exists() {
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(config_path)
                .with_context(|| format!("failed to read {}", config_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", config_path.display()))?
    } else {
        json!({
            "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
            "skillDir": default_skill_dir,
            "dependencies": {}
        })
    };
    let object = value
        .as_object_mut()
        .context("config root must be a JSON object")?;
    object.insert("defaultAgents".to_owned(), json!(agents));
    let bytes = format!("{}\n", serde_json::to_string_pretty(&value)?);
    write_atomic(config_path, bytes.as_bytes(), WriteMode::Replace)
        .map(|_| ())
        .with_context(|| format!("failed to write {}", config_path.display()))
}

fn default_skill_dir_for_scope(scope: ConfigScope) -> &'static str {
    if scope.is_global() {
        "~/.sksync/skills"
    } else {
        "./.sksync/skills"
    }
}

#[cfg(test)]
mod tests {
    use super::write_default_agents_config;

    #[test]
    fn default_agents_symlink_preserves_fields_permissions_and_old_inode() {
        use std::io::Read;
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let referent = dir.path().join("dotfiles.json");
        let original = br#"{ "skillDir": "./skills", "unknown": 42, "dependencies": { "review": {"source": "./review", "agents": ["pi"], "managedByBundles": true} } }"#;
        std::fs::write(&referent, original).unwrap();
        std::fs::set_permissions(&referent, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&referent, &path).unwrap();
        let mut previous = std::fs::File::open(&path).unwrap();
        let failed = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            write_default_agents_config(&path, "./ignored", &["universal".into()])
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        write_default_agents_config(&path, "./ignored", &["universal".into()]).unwrap();
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let mut bytes = Vec::new();
        previous.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, original);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["unknown"], 42);
        assert_eq!(value["skillDir"], "./skills");
        assert_eq!(value["dependencies"]["review"]["managedByBundles"], true);
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi"])
        );
        assert_eq!(value["defaultAgents"], serde_json::json!(["universal"]));
    }

    #[test]
    fn default_agents_failed_publication_preserves_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let result = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            write_default_agents_config(&path, "./skills", &[])
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn default_agents_failed_publication_preserves_raw_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let original = br#"{ "unknown": 42, "dependencies": {}, "defaultAgents": ["pi"] }"#;
        std::fs::write(&path, original).unwrap();
        let result = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            write_default_agents_config(&path, "./skills", &["universal".into()])
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn write_default_agents_config_creates_missing_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");

        write_default_agents_config(
            &config_path,
            "./.sksync/skills",
            &["universal".to_owned(), "pi".to_owned()],
        )
        .expect("write defaults");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(value["skillDir"], "./.sksync/skills");
        assert_eq!(value["dependencies"], serde_json::json!({}));
        assert_eq!(
            value["defaultAgents"],
            serde_json::json!(["universal", "pi"])
        );
    }

    #[test]
    fn write_default_agents_config_preserves_existing_config_fields() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "skillDir": "skills",
              "dependencies": {
                "review": { "source": "./review", "agents": ["pi"] }
              }
            }"#,
        )
        .expect("write config");

        write_default_agents_config(&config_path, "./.sksync/skills", &["universal".to_owned()])
            .expect("write defaults");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(value["skillDir"], "skills");
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi"])
        );
        assert_eq!(value["defaultAgents"], serde_json::json!(["universal"]));
    }
}

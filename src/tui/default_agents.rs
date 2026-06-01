use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::config::{config_path_for_scope, load_optional_config_for_scope, ConfigScope};
use super::{prompt_config_scope, prompt_default_agents};
use crate::application::config::ResolvedConfig;

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
    std::fs::write(
        config_path,
        format!("{}\n", serde_json::to_string_pretty(&value)?),
    )
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

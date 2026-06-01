use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use inquire::Select;

use super::commands::{bundle_add_args, bundle_remove_args};
use super::{
    config_path_for_scope, confirm_and_run, default_agents_from_config, global_config_root,
    load_optional_config_for_scope, prompt_agents, prompt_config_scope, prompt_confirm,
    prompt_required, run_sksync,
};
use crate::application::bundle::load_bundle_from_source;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BundleProvenanceChoice {
    pub(super) name: String,
    pub(super) source: String,
}

impl fmt::Display for BundleProvenanceChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} — {}", self.name, self.source)
    }
}

pub(super) fn run_add(project_root: &Path) -> Result<()> {
    let source = prompt_required("Bundle source")?;
    let scope = prompt_config_scope("Where should this bundle be added?")?;
    let root_dir = if scope.is_global() {
        global_config_root()?
    } else {
        project_root.to_path_buf()
    };
    let bundle = load_bundle_from_source(&source, &root_dir)?;

    println!("Bundle");
    println!("Name: {}", bundle.manifest.name);
    println!("Description: {}", bundle.manifest.description);
    println!("Source: {}", bundle.provenance.source);
    println!("Entries ({})", bundle.entries.len());
    for entry in &bundle.entries {
        println!(
            "- {}: {} -> {}",
            entry.skill_name, entry.original_source, entry.normalized_source
        );
    }
    if !prompt_confirm("Continue with this bundle?", true)? {
        return Ok(());
    }

    let config = load_optional_config_for_scope(project_root, scope)?;
    let default_agents = default_agents_from_config(config.as_ref());
    let agents = prompt_agents(scope, config.as_ref(), &default_agents)?;
    let dry_run_args = bundle_add_args(&source, &agents, scope.is_global(), true);
    println!("dry-run plan:");
    run_sksync(project_root, &dry_run_args)?;

    let apply_args = bundle_add_args(&source, &agents, scope.is_global(), false);
    confirm_and_run(project_root, "Add this bundle?", apply_args)
}

pub(super) fn run_remove(project_root: &Path) -> Result<()> {
    let scope = prompt_config_scope("Which config should the bundle be removed from?")?;
    let config_path = config_path_for_scope(project_root, scope)?;
    let choices = bundle_provenance_choices_from_config_path(&config_path)?;
    if choices.is_empty() {
        println!(
            "No bundle provenance is configured in {}",
            config_path.display()
        );
        return Ok(());
    }
    let choice = Select::new("Select bundle provenance to remove", choices)
        .prompt()
        .context("failed to read bundle provenance selection")?;
    let dry_run_args = bundle_remove_args(&choice, scope.is_global(), true);
    println!("dry-run plan:");
    run_sksync(project_root, &dry_run_args)?;

    let apply_args = bundle_remove_args(&choice, scope.is_global(), false);
    confirm_and_run(project_root, "Remove this bundle provenance?", apply_args)
}

fn bundle_provenance_choices_from_config_path(
    config_path: &Path,
) -> Result<Vec<BundleProvenanceChoice>> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let value = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", config_path.display()))?;
    Ok(bundle_provenance_choices_from_value(&value))
}

fn bundle_provenance_choices_from_value(value: &serde_json::Value) -> Vec<BundleProvenanceChoice> {
    let mut choices = BTreeSet::new();
    let Some(dependencies) = value
        .get("dependencies")
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };
    for dependency in dependencies.values() {
        let Some(bundles) = dependency.get("bundles").and_then(|value| value.as_array()) else {
            continue;
        };
        for bundle in bundles {
            let (Some(name), Some(source)) = (
                bundle.get("name").and_then(|value| value.as_str()),
                bundle.get("source").and_then(|value| value.as_str()),
            ) else {
                continue;
            };
            choices.insert(BundleProvenanceChoice {
                name: name.to_owned(),
                source: source.to_owned(),
            });
        }
    }
    choices.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{bundle_provenance_choices_from_value, BundleProvenanceChoice};

    #[test]
    fn bundle_provenance_choices_are_unique_exact_name_source_pairs() {
        let value = serde_json::json!({
            "dependencies": {
                "review": {
                    "bundles": [
                        { "name": "baseline", "source": "./bundle-a" },
                        { "name": "baseline", "source": "./bundle-a" }
                    ]
                },
                "qa": {
                    "bundles": [
                        { "name": "baseline", "source": "./bundle-b" }
                    ]
                }
            }
        });

        let choices = bundle_provenance_choices_from_value(&value);

        assert_eq!(
            choices,
            vec![
                BundleProvenanceChoice {
                    name: "baseline".to_owned(),
                    source: "./bundle-a".to_owned(),
                },
                BundleProvenanceChoice {
                    name: "baseline".to_owned(),
                    source: "./bundle-b".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn bundle_provenance_choices_ignore_missing_or_invalid_entries() {
        let value = serde_json::json!({
            "dependencies": {
                "no-bundles": {},
                "bad-bundles": { "bundles": [
                    { "name": "baseline" },
                    { "source": "./bundle" },
                    "not-object"
                ] },
                "valid": { "bundles": [
                    { "name": "baseline", "source": "./bundle" }
                ] }
            }
        });

        assert_eq!(
            bundle_provenance_choices_from_value(&value),
            vec![BundleProvenanceChoice {
                name: "baseline".to_owned(),
                source: "./bundle".to_owned(),
            }]
        );
    }
}

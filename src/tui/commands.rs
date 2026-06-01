use super::add_skill::PackageFilterChoice;
use super::{BundleProvenanceChoice, ConfigScope};

pub(super) fn add_skill_args(
    source: &str,
    name: &str,
    agents: &[String],
    scope: ConfigScope,
    package_filter: &PackageFilterChoice,
) -> Vec<String> {
    let mut args = vec!["add".to_owned(), source.to_owned()];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent.clone());
    }
    if !name.trim().is_empty() {
        args.push("--name".to_owned());
        args.push(name.trim().to_owned());
    }
    if scope.is_global() {
        args.push("--global".to_owned());
    }
    match package_filter {
        PackageFilterChoice::FullPackage => {}
        PackageFilterChoice::ManifestOnly => args.push("--manifest-only".to_owned()),
        PackageFilterChoice::Custom(patterns) => {
            for pattern in patterns {
                args.push("--include".to_owned());
                args.push(pattern.clone());
            }
        }
    }
    args
}

pub(super) fn bundle_add_args(
    source: &str,
    agents: &[String],
    global: bool,
    dry_run: bool,
) -> Vec<String> {
    let mut args = vec!["bundle".to_owned(), "add".to_owned(), source.to_owned()];
    for agent in agents {
        args.push("--agent".to_owned());
        args.push(agent.clone());
    }
    if global {
        args.push("--global".to_owned());
    }
    if dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

pub(super) fn bundle_remove_args(
    choice: &BundleProvenanceChoice,
    global: bool,
    dry_run: bool,
) -> Vec<String> {
    let mut args = vec![
        "bundle".to_owned(),
        "remove".to_owned(),
        choice.name.clone(),
        "--source".to_owned(),
        choice.source.clone(),
    ];
    if global {
        args.push("--global".to_owned());
    }
    if dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::{add_skill_args, bundle_add_args, bundle_remove_args};
    use crate::tui::add_skill::PackageFilterChoice;
    use crate::tui::{BundleProvenanceChoice, ConfigScope};

    #[test]
    fn add_skill_args_omit_include_flags_for_full_package() {
        assert_eq!(
            add_skill_args(
                "owner/repo",
                "",
                &["pi".to_owned()],
                ConfigScope::Project,
                &PackageFilterChoice::FullPackage,
            ),
            vec!["add", "owner/repo", "--agent", "pi"]
        );
    }

    #[test]
    fn add_skill_args_include_manifest_only_flag() {
        assert_eq!(
            add_skill_args(
                "ogulcancelik/herdr",
                "herdr",
                &["pi".to_owned()],
                ConfigScope::Project,
                &PackageFilterChoice::ManifestOnly,
            ),
            vec![
                "add",
                "ogulcancelik/herdr",
                "--agent",
                "pi",
                "--name",
                "herdr",
                "--manifest-only",
            ]
        );
    }

    #[test]
    fn add_skill_args_include_custom_patterns() {
        assert_eq!(
            add_skill_args(
                "org/repo/skills/review",
                "",
                &["pi".to_owned(), "claude-code".to_owned()],
                ConfigScope::Global,
                &PackageFilterChoice::Custom(vec!["SKILL.md".to_owned(), "references".to_owned()]),
            ),
            vec![
                "add",
                "org/repo/skills/review",
                "--agent",
                "pi",
                "--agent",
                "claude-code",
                "--global",
                "--include",
                "SKILL.md",
                "--include",
                "references",
            ]
        );
    }

    #[test]
    fn bundle_add_args_include_agents_scope_and_dry_run() {
        assert_eq!(
            bundle_add_args(
                "./bundle",
                &["pi".to_owned(), "claude-code".to_owned()],
                true,
                true,
            ),
            vec![
                "bundle",
                "add",
                "./bundle",
                "--agent",
                "pi",
                "--agent",
                "claude-code",
                "--global",
                "--dry-run"
            ]
        );
    }

    #[test]
    fn bundle_remove_args_include_exact_source_scope_and_dry_run() {
        let choice = BundleProvenanceChoice {
            name: "review-workflow".to_owned(),
            source: "./bundle".to_owned(),
        };

        assert_eq!(
            bundle_remove_args(&choice, true, true),
            vec![
                "bundle",
                "remove",
                "review-workflow",
                "--source",
                "./bundle",
                "--global",
                "--dry-run"
            ]
        );
    }
}

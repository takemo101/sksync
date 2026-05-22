use std::path::{Path, PathBuf};

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

impl InstallSource {
    pub fn storage_subpath(&self, skill_name: &str) -> PathBuf {
        match self {
            Self::Git(source) => source.storage_subpath(skill_name),
            Self::Local(_) => PathBuf::from(skill_name),
        }
    }
}

impl GitInstallSource {
    pub fn wanted_ref(&self) -> &str {
        self.reference.as_deref().unwrap_or("HEAD")
    }

    pub fn storage_subpath(&self, skill_name: &str) -> PathBuf {
        let mut subpath = repo_storage_prefix(&self.url);
        let skill_path = skill_storage_path(&self.path, skill_name);
        if skill_path.as_os_str().is_empty() {
            subpath.push(skill_name);
        } else {
            subpath.push(skill_path);
        }
        subpath
    }
}

fn repo_storage_prefix(url: &str) -> PathBuf {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = split_git_url(trimmed);
    let path_parts = path
        .split('/')
        .filter(|part| is_safe_storage_component(part))
        .collect::<Vec<_>>();

    let mut parts = Vec::new();
    if host.is_some_and(|host| host != "github.com") {
        parts.push(host.expect("host checked above"));
    }
    parts.extend(path_parts);

    if parts.is_empty() {
        PathBuf::from("unknown-source")
    } else {
        parts.iter().collect()
    }
}

fn split_git_url(value: &str) -> (Option<&str>, &str) {
    if let Some(rest) = value.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return (Some(host), path.trim_start_matches('/'));
        }
    }

    let without_scheme = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
        .unwrap_or(value);
    let without_user = without_scheme
        .split_once('@')
        .map_or(without_scheme, |(_, rest)| rest);
    if let Some((host, path)) = without_user.split_once('/') {
        (Some(host), path)
    } else {
        (None, without_user)
    }
}

fn skill_storage_path(path: &Path, skill_name: &str) -> PathBuf {
    let components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter(|component| *component != "." && is_safe_storage_component(component))
        .collect::<Vec<_>>();
    let components = components
        .strip_prefix(&["skills"])
        .unwrap_or(components.as_slice());

    match components.split_last() {
        Some((_name, parents)) if !parents.is_empty() => {
            let mut path = parents.iter().collect::<PathBuf>();
            path.push(skill_name);
            path
        }
        Some(_) | None => PathBuf::from(skill_name),
    }
}

fn is_safe_storage_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".."
}

#[cfg(test)]
mod tests {
    use super::{GitInstallSource, InstallSource};
    use std::path::{Path, PathBuf};

    #[test]
    fn git_source_storage_subpath_uses_repo_and_skill_path() {
        let source = InstallSource::Git(GitInstallSource {
            url: "https://github.com/obra/superpowers.git".to_owned(),
            reference: None,
            path: PathBuf::from("skills/brainstorming"),
        });

        assert_eq!(
            source.storage_subpath("brainstorming"),
            Path::new("obra/superpowers/brainstorming")
        );
    }

    #[test]
    fn git_source_storage_subpath_falls_back_to_skill_name_for_repo_root() {
        let source = InstallSource::Git(GitInstallSource {
            url: "https://github.com/owner/repo.git".to_owned(),
            reference: None,
            path: PathBuf::from("."),
        });

        assert_eq!(
            source.storage_subpath("review"),
            Path::new("owner/repo/review")
        );
    }

    #[test]
    fn git_source_storage_subpath_uses_skill_name_as_leaf() {
        let source = InstallSource::Git(GitInstallSource {
            url: "https://github.com/owner/repo.git".to_owned(),
            reference: Some("v1".to_owned()),
            path: PathBuf::from("skills/review"),
        });

        assert_eq!(
            source.storage_subpath("review-v1"),
            Path::new("owner/repo/review-v1")
        );
    }

    #[test]
    fn non_github_source_storage_subpath_keeps_host_and_repo_name() {
        let source = InstallSource::Git(GitInstallSource {
            url: "https://gitlab.com/team/repo-a.git".to_owned(),
            reference: None,
            path: PathBuf::from("skills/review"),
        });

        assert_eq!(
            source.storage_subpath("review"),
            Path::new("gitlab.com/team/repo-a/review")
        );
    }

    #[test]
    fn scp_like_source_storage_subpath_keeps_non_github_host_and_repo_name() {
        let source = InstallSource::Git(GitInstallSource {
            url: "git@gitlab.com:team/repo-a.git".to_owned(),
            reference: None,
            path: PathBuf::from("skills/review"),
        });

        assert_eq!(
            source.storage_subpath("review"),
            Path::new("gitlab.com/team/repo-a/review")
        );
    }

    #[test]
    fn local_source_storage_subpath_preserves_legacy_skill_name_layout() {
        let source = InstallSource::Local(PathBuf::from("../vendor/review"));

        assert_eq!(source.storage_subpath("review"), Path::new("review"));
    }
}

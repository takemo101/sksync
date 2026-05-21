use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SkillManifestError {
    #[error("SKILL.md YAML frontmatter is missing")]
    MissingFrontmatter,
    #[error("SKILL.md YAML frontmatter is invalid: {0}")]
    InvalidFrontmatter(String),
    #[error("SKILL.md frontmatter field '{field}' is required")]
    MissingRequiredField { field: &'static str },
    #[error("SKILL.md frontmatter field '{field}' must be a string")]
    RequiredFieldNotString { field: &'static str },
    #[error("SKILL.md frontmatter field '{field}' must not be empty")]
    RequiredFieldEmpty { field: &'static str },
}

pub fn parse_skill_manifest(content: &str) -> Result<SkillManifest, SkillManifestError> {
    let frontmatter =
        extract_yaml_frontmatter(content).ok_or(SkillManifestError::MissingFrontmatter)?;
    let frontmatter = serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
        .map_err(|error| SkillManifestError::InvalidFrontmatter(error.to_string()))?;

    Ok(SkillManifest {
        name: required_frontmatter_string(&frontmatter, "name")?,
        description: required_frontmatter_string(&frontmatter, "description")?,
    })
}

fn extract_yaml_frontmatter(content: &str) -> Option<&str> {
    let content = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;
    let end = content
        .find("\n---\n")
        .or_else(|| content.find("\n---\r\n"))?;
    Some(&content[..end])
}

fn required_frontmatter_string(
    frontmatter: &serde_yaml::Value,
    field: &'static str,
) -> Result<String, SkillManifestError> {
    let value = frontmatter
        .get(field)
        .ok_or(SkillManifestError::MissingRequiredField { field })?;
    let value = value
        .as_str()
        .ok_or(SkillManifestError::RequiredFieldNotString { field })?;
    if value.trim().is_empty() {
        return Err(SkillManifestError::RequiredFieldEmpty { field });
    }
    Ok(value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{parse_skill_manifest, SkillManifest, SkillManifestError};

    #[test]
    fn parses_required_skill_manifest_fields() {
        let manifest =
            parse_skill_manifest("---\nname: review\ndescription: Review helper\n---\n# Review\n")
                .expect("manifest parses");

        assert_eq!(
            manifest,
            SkillManifest {
                name: "review".to_owned(),
                description: "Review helper".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_missing_frontmatter() {
        assert_eq!(
            parse_skill_manifest("# Review\n"),
            Err(SkillManifestError::MissingFrontmatter)
        );
    }

    #[test]
    fn rejects_empty_required_field() {
        assert_eq!(
            parse_skill_manifest("---\nname: review\ndescription: ' '\n---\n"),
            Err(SkillManifestError::RequiredFieldEmpty {
                field: "description"
            })
        );
    }
}

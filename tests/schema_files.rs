use serde_json::Value;

const CONFIG_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json";
const AGENTS_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json";

#[test]
fn schema_files_are_valid_json() {
    parse_json(include_str!("../schemas/sksync.schema.json"));
    parse_json(include_str!("../schemas/sksync.agents.schema.json"));
}

#[test]
fn examples_point_to_repository_schemas() {
    let config = parse_json(include_str!("../sksync.config.example.json"));
    let agents = parse_json(include_str!("../sksync.agents.example.json"));

    assert_eq!(config["$schema"], CONFIG_SCHEMA_ID);
    assert_eq!(agents["$schema"], AGENTS_SCHEMA_ID);
}

#[test]
fn schema_ids_match_example_references() {
    let config_schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let agents_schema = parse_json(include_str!("../schemas/sksync.agents.schema.json"));

    assert_eq!(config_schema["$id"], CONFIG_SCHEMA_ID);
    assert_eq!(agents_schema["$id"], AGENTS_SCHEMA_ID);
}

#[test]
fn config_schema_covers_supported_top_level_fields() {
    let schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let properties = schema["properties"].as_object().expect("properties object");

    for field in ["$schema", "skillDir", "agents", "skills", "dependencies"] {
        assert!(properties.contains_key(field), "missing field {field}");
    }
}

#[test]
fn config_schema_structured_sources_match_runtime_requirements() {
    let schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let structured_source = &schema["$defs"]["structuredInstallSource"];
    let git_source = &schema["$defs"]["structuredGitInstallSource"];
    let local_source = &schema["$defs"]["structuredLocalInstallSource"];

    assert_eq!(
        structured_source["oneOf"],
        serde_json::json!([
            { "$ref": "#/$defs/structuredGitInstallSource" },
            { "$ref": "#/$defs/structuredLocalInstallSource" }
        ])
    );
    assert_eq!(
        git_source["anyOf"],
        serde_json::json!([{ "required": ["url"] }, { "required": ["repo"] }])
    );
    assert_eq!(
        local_source["required"],
        serde_json::json!(["provider", "path"])
    );
    assert_eq!(local_source["properties"]["provider"]["const"], "local");
}

#[test]
fn agents_schema_requires_agent_targets() {
    let schema = parse_json(include_str!("../schemas/sksync.agents.schema.json"));
    assert_eq!(
        schema["anyOf"],
        serde_json::json!([
            { "required": ["global"] },
            { "required": ["project"] }
        ])
    );
    assert_eq!(
        schema["$defs"]["agentTargetMapping"]["required"],
        serde_json::json!(["targetDir"])
    );
}

#[test]
fn agents_example_uses_documented_skill_directories() {
    let agents = parse_json(include_str!("../sksync.agents.example.json"));

    assert_eq!(
        agents["global"]["antigravity"]["targetDir"],
        "~/.gemini/antigravity/skills"
    );
    assert_eq!(
        agents["project"]["antigravity"]["targetDir"],
        ".agents/skills"
    );
    assert_eq!(agents["global"]["jcode"]["targetDir"], "~/.jcode/skills");
    assert_eq!(agents["project"]["jcode"]["targetDir"], ".jcode/skills");
    assert_eq!(
        agents["global"]["universal"]["targetDir"],
        "~/.agents/skills"
    );
    assert_eq!(
        agents["project"]["universal"]["targetDir"],
        ".agents/skills"
    );
}

#[test]
fn agents_example_includes_skillkit_compatible_mappings() {
    let agents = parse_json(include_str!("../sksync.agents.example.json"));
    let mappings = agents["global"].as_object().expect("global object");
    let project_mappings = agents["project"].as_object().expect("project object");

    for agent in [
        "claude-code",
        "cursor",
        "codex",
        "gemini-cli",
        "opencode",
        "github-copilot",
        "jcode",
        "universal",
        "windsurf",
        "roo",
        "aider",
        "hermes",
    ] {
        assert!(mappings.contains_key(agent), "missing mapping for {agent}");
        assert!(
            project_mappings.contains_key(agent),
            "missing project mapping for {agent}"
        );
    }

    assert!(
        mappings.len() >= 46,
        "expected SkillKit-compatible agent coverage"
    );
    assert!(
        project_mappings.len() >= 46,
        "expected SkillKit-compatible project agent coverage"
    );
}

fn parse_json(content: &str) -> Value {
    serde_json::from_str(content).expect("valid JSON")
}

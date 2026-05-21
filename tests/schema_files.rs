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
fn agents_schema_requires_agent_targets() {
    let schema = parse_json(include_str!("../schemas/sksync.agents.schema.json"));
    assert_eq!(schema["required"], serde_json::json!(["agents"]));
    assert_eq!(
        schema["$defs"]["agentTargetMapping"]["required"],
        serde_json::json!(["targetDir"])
    );
}

#[test]
fn agents_example_includes_skillkit_compatible_mappings() {
    let agents = parse_json(include_str!("../sksync.agents.example.json"));
    let mappings = agents["agents"].as_object().expect("agents object");

    for agent in [
        "claude-code",
        "cursor",
        "codex",
        "gemini-cli",
        "opencode",
        "github-copilot",
        "windsurf",
        "roo",
        "aider",
        "hermes",
    ] {
        assert!(mappings.contains_key(agent), "missing mapping for {agent}");
    }

    assert!(
        mappings.len() >= 46,
        "expected SkillKit-compatible agent coverage"
    );
}

fn parse_json(content: &str) -> Value {
    serde_json::from_str(content).expect("valid JSON")
}

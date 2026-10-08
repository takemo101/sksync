use serde_json::Value;

const CONFIG_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json";
const AGENTS_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.agents.schema.json";
const LOCK_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync-lock.schema.json";
const BUNDLE_SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json";

#[test]
fn schema_files_are_valid_json() {
    parse_json(include_str!("../schemas/sksync.schema.json"));
    parse_json(include_str!("../schemas/sksync.agents.schema.json"));
    parse_json(include_str!("../schemas/sksync-lock.schema.json"));
    parse_json(include_str!("../schemas/sksync.bundle.schema.json"));
}

#[test]
fn examples_point_to_repository_schemas() {
    let config = parse_json(include_str!("../sksync.config.example.json"));
    let agents = parse_json(include_str!("../sksync.agents.example.json"));
    let lockfile = parse_json(include_str!("../sksync-lock.example.json"));
    let bundle = parse_json(include_str!("../sksync.bundle.example.json"));

    assert_eq!(config["$schema"], CONFIG_SCHEMA_ID);
    assert_eq!(agents["$schema"], AGENTS_SCHEMA_ID);
    assert_eq!(lockfile["$schema"], LOCK_SCHEMA_ID);
    assert_eq!(bundle["$schema"], BUNDLE_SCHEMA_ID);
}

#[test]
fn schema_ids_match_example_references() {
    let config_schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let agents_schema = parse_json(include_str!("../schemas/sksync.agents.schema.json"));
    let lock_schema = parse_json(include_str!("../schemas/sksync-lock.schema.json"));
    let bundle_schema = parse_json(include_str!("../schemas/sksync.bundle.schema.json"));

    assert_eq!(config_schema["$id"], CONFIG_SCHEMA_ID);
    assert_eq!(agents_schema["$id"], AGENTS_SCHEMA_ID);
    assert_eq!(lock_schema["$id"], LOCK_SCHEMA_ID);
    assert_eq!(bundle_schema["$id"], BUNDLE_SCHEMA_ID);
}

#[test]
fn config_schema_covers_supported_top_level_fields() {
    let schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let properties = schema["properties"].as_object().expect("properties object");

    for field in [
        "$schema",
        "skillDir",
        "agents",
        "defaultAgents",
        "skills",
        "dependencies",
    ] {
        assert!(properties.contains_key(field), "missing field {field}");
    }
}

#[test]
fn config_schema_allows_bundle_provenance_on_dependencies() {
    let schema = parse_json(include_str!("../schemas/sksync.schema.json"));
    let dependency = &schema["$defs"]["dependencyConfig"];

    assert!(dependency["properties"].get("bundles").is_some());
    assert!(dependency["properties"].get("managedByBundles").is_some());
    assert_eq!(
        dependency["properties"]["bundles"]["items"],
        serde_json::json!({ "$ref": "#/$defs/bundleProvenance" })
    );
    assert_eq!(
        dependency["properties"]["managedByBundles"]["default"],
        false
    );
}

#[test]
fn bundle_schema_rejects_unknown_entry_fields() {
    let schema = parse_json(include_str!("../schemas/sksync.bundle.schema.json"));
    let entry = &schema["$defs"]["bundleEntry"];

    assert_eq!(entry["additionalProperties"], false);
    assert_eq!(entry["required"], serde_json::json!(["source"]));
    assert!(entry["properties"].get("include").is_some());
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
fn lock_schema_covers_current_portable_v5_fields() {
    let schema = parse_json(include_str!("../schemas/sksync-lock.schema.json"));

    assert_eq!(schema["properties"]["lockfileVersion"]["const"], 5);
    assert_eq!(schema["properties"]["root"]["const"], ".");
    assert_eq!(
        schema["$defs"]["lockedSkill"]["required"],
        serde_json::json!(["source", "hash", "files"])
    );
    assert!(schema["$defs"]["lockedSkill"]["properties"]
        .get("include")
        .is_some());
    assert_eq!(
        schema["$defs"]["lockedGitInstallSource"]["required"],
        serde_json::json!(["type", "url", "path"])
    );
    assert_eq!(
        schema["$defs"]["lockedLocalInstallSource"]["required"],
        serde_json::json!(["type", "path"])
    );
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
        agents["global"]["codewhale"]["targetDir"],
        "~/.codewhale/skills"
    );
    assert_eq!(
        agents["project"]["codewhale"]["targetDir"],
        ".codewhale/skills"
    );
    assert_eq!(agents["global"]["fx"]["targetDir"], "~/.fx/skills");
    assert_eq!(agents["project"]["fx"]["targetDir"], ".agents/skills");
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
        "codewhale",
        "cursor",
        "codex",
        "gemini-cli",
        "fx",
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
        mappings.len() >= 47,
        "expected SkillKit-compatible agent coverage"
    );
    assert!(
        project_mappings.len() >= 47,
        "expected SkillKit-compatible project agent coverage"
    );
}

#[test]
fn skill_property_names_exclude_reserved_components_in_both_schemas() {
    let config = parse_json(include_str!("../schemas/sksync.schema.json"));
    let bundle = parse_json(include_str!("../schemas/sksync.bundle.schema.json"));
    let expected = serde_json::json!({
        "type": "string",
        "minLength": 1,
        "pattern": "^[^\\s/\\\\](?:[^/\\\\]*[^\\s/\\\\])?$",
        "not": { "pattern": "^\\s*\\.{1,2}\\s*$" }
    });
    assert_eq!(config["$defs"]["skillName"], expected);
    for field in ["skills", "dependencies"] {
        assert_eq!(
            config["properties"][field]["propertyNames"],
            serde_json::json!({ "$ref": "#/$defs/skillName" })
        );
    }
    assert_eq!(bundle["properties"]["entries"]["propertyNames"], expected);
    // Bundle names are provenance labels, not managed-body components.
    assert!(bundle["properties"]["name"].get("not").is_none());
}

fn parse_json(content: &str) -> Value {
    serde_json::from_str(content).expect("valid JSON")
}

#[test]
fn output_schema_v1_covers_success_partial_and_aborted_reports_for_every_command() {
    let schema = parse_json(include_str!("../schemas/sksync-output.schema.json"));
    assert_eq!(
        schema["$id"],
        "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync-output.schema.json"
    );
    assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    let error = serde_json::json!({"code": "IO_ERROR", "message": "Failed.", "hint": "Try again."});
    let reports = [
        (
            "list",
            serde_json::json!({
                "skills": [{"name": "review", "source": "/tmp/review", "installSource": null,
                    "include": null, "lockedHash": null, "targets": []}], "lockfileStatus": "missing"
            }),
        ),
        ("plan", serde_json::json!({"items": [], "applicable": true})),
        (
            "check",
            serde_json::json!({"healthy": true, "problems": []}),
        ),
        ("outdated", serde_json::json!({"rows": [], "problems": []})),
    ];
    for (command, data) in reports {
        for scope in ["project", "global"] {
            for (ok, report) in [
                (true, data.clone()),
                (false, data.clone()),
                (false, Value::Null),
            ] {
                let response = serde_json::json!({"schemaVersion": 1, "command": command,
                    "scope": scope, "ok": ok, "data": report, "error": if ok { Value::Null } else { error.clone() }});
                assert!(
                    output_schema_matches(&schema, &schema, &response),
                    "{response}"
                );
                let mut missing = response.clone();
                missing.as_object_mut().unwrap().remove("data");
                assert!(!output_schema_matches(&schema, &schema, &missing));
                let mut wrong_shape = response.clone();
                wrong_shape["data"] = serde_json::json!({"wrongCommandReport": []});
                assert!(!output_schema_matches(&schema, &schema, &wrong_shape));
            }
        }
    }
}

#[test]
fn output_schema_fixtures_cover_all_tags_required_nulls_and_variant_fields() {
    let schema = parse_json(include_str!("../schemas/sksync-output.schema.json"));
    let error = serde_json::json!({"code": "INSPECTION_FAILED", "message": "Cannot inspect."});
    for status in [
        "synced",
        "missing",
        "drifted",
        "conflict",
        "brokenSymlink",
        "sourceMissing",
        "resolveFailed",
        "inspectFailed",
    ] {
        let mut target =
            serde_json::json!({"agent": "pi", "target": "/tmp/target", "status": status});
        if matches!(status, "resolveFailed" | "inspectFailed") {
            target["error"] = error.clone();
        }
        if status == "resolveFailed" {
            target["target"] = Value::Null;
        }
        assert!(output_schema_matches(
            &schema,
            &schema["$defs"]["listTarget"],
            &target
        ));
        target["status"] = "unknown".into();
        assert!(!output_schema_matches(
            &schema,
            &schema["$defs"]["listTarget"],
            &target
        ));
    }
    for source in [
        serde_json::json!({"type":"local", "path":"./source"}),
        serde_json::json!({"type":"git", "url":"./repo.git", "path":"skills/review", "ref":"main"}),
    ] {
        let skill = serde_json::json!({"name":"review", "source":"/tmp/review", "installSource":source,
            "include":["SKILL.md"], "lockedHash":"abc", "targets":[]});
        assert!(output_schema_matches(
            &schema,
            &schema["$defs"]["listSkill"],
            &skill
        ));
        for field in ["installSource", "include", "lockedHash", "targets"] {
            let mut missing = skill.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(!output_schema_matches(
                &schema,
                &schema["$defs"]["listSkill"],
                &missing
            ));
        }
    }
    for action in [
        "createSymlink",
        "alreadySynced",
        "conflict",
        "driftedSymlink",
        "sourceMissing",
    ] {
        let mut item = serde_json::json!({"owners":[{"skill":"review", "agent":"pi"},{"skill":"review", "agent":"fx"}],
            "source":"/tmp/source", "target":"/tmp/target", "action":action});
        if action == "driftedSymlink" {
            item["actualSource"] = "/tmp/other".into();
        }
        if action == "conflict" {
            for reason in ["regularFile", "directory", "brokenSymlink"] {
                item["reason"] = reason.into();
                assert!(output_schema_matches(
                    &schema,
                    &schema["$defs"]["planItem"],
                    &item
                ));
            }
            item["reason"] = "regularFile".into();
        }
        assert!(output_schema_matches(
            &schema,
            &schema["$defs"]["planItem"],
            &item
        ));
    }
    for (kind, fields) in [
        ("sourceHashDrift", vec!["skill", "expected", "actual"]),
        ("targetMissing", vec!["skill", "agent", "path"]),
        (
            "targetUnexpectedSymlink",
            vec!["skill", "agent", "path", "actualSource"],
        ),
        (
            "brokenSymlink",
            vec!["skill", "agent", "path", "actualSource"],
        ),
        ("targetConflict", vec!["skill", "agent", "path", "reason"]),
        ("inspectFailed", vec!["skill", "agent", "message"]),
        ("hashFailed", vec!["skill", "message"]),
        ("includeMismatch", vec!["skill", "expected", "actual"]),
    ] {
        let mut problem = serde_json::json!({"kind":kind});
        for field in &fields {
            problem[field] = "fixture".into();
        }
        assert!(output_schema_matches(
            &schema,
            &schema["$defs"]["checkProblem"],
            &problem
        ));
        for field in &fields {
            let mut missing = problem.clone();
            missing.as_object_mut().unwrap().remove(*field);
            assert!(!output_schema_matches(
                &schema,
                &schema["$defs"]["checkProblem"],
                &missing
            ));
        }
    }
    let partial = serde_json::json!({"schemaVersion":1,"command":"outdated","scope":"project","ok":false,
        "data":{"rows":[{"skill":"review","current":"old","wanted":"main","latest":"new","source":"./repo.git","status":"outdated"}],
            "problems":[{"skill":"qa","source":"./missing.git","wanted":"main","error":{"code":"REMOTE_QUERY_FAILED","message":"Query failed."}}]},
        "error":{"code":"REMOTE_QUERY_FAILED","message":"A query failed."}});
    assert!(output_schema_matches(&schema, &schema, &partial));
    let mut invalid = partial.clone();
    invalid["data"]["rows"] = Value::Null;
    assert!(!output_schema_matches(&schema, &schema, &invalid));
    invalid = partial.clone();
    invalid["ok"] = true.into();
    assert!(!output_schema_matches(&schema, &schema, &invalid));
    invalid = partial.clone();
    invalid["error"] = Value::Null;
    assert!(!output_schema_matches(&schema, &schema, &invalid));
    invalid = partial.clone();
    invalid["schemaVersion"] = 2.into();
    assert!(!output_schema_matches(&schema, &schema, &invalid));
    // Compatible optional additions do not invalidate older consumers' contracts.
    invalid = partial;
    invalid["futureField"] = true.into();
    assert!(output_schema_matches(&schema, &schema, &invalid));
}

// Test-only evaluator for the published output schema's keyword subset. Reject
// unhandled keywords so future schema edits cannot silently weaken fixture checks.
fn output_schema_matches(root: &Value, schema: &Value, value: &Value) -> bool {
    for keyword in schema.as_object().expect("schema object").keys() {
        assert!(
            [
                "$schema",
                "$id",
                "$defs",
                "title",
                "description",
                "$ref",
                "type",
                "const",
                "enum",
                "required",
                "properties",
                "items",
                "allOf",
                "oneOf",
                "anyOf",
                "minLength"
            ]
            .contains(&keyword.as_str()),
            "unsupported schema keyword {keyword}"
        );
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return output_schema_matches(
            root,
            root.pointer(reference.strip_prefix('#').unwrap())
                .expect("local schema reference"),
            value,
        );
    }
    if let Some(expected) = schema.get("const") {
        if value != expected {
            return false;
        }
    }
    if let Some(variants) = schema.get("enum").and_then(Value::as_array) {
        if !variants.contains(value) {
            return false;
        }
    }
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let valid = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            "integer" => value.is_i64() || value.is_u64(),
            _ => panic!("unsupported schema type {kind}"),
        };
        if !valid {
            return false;
        }
    }
    if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
        if value
            .as_str()
            .is_some_and(|s| (s.chars().count() as u64) < min)
        {
            return false;
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        if value.is_object()
            && required
                .iter()
                .any(|key| value.get(key.as_str().unwrap()).is_none())
        {
            return false;
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, property) in properties {
            if let Some(field) = value.get(key) {
                if !output_schema_matches(root, property, field) {
                    return false;
                }
            }
        }
    }
    if let (Some(item_schema), Some(items)) = (schema.get("items"), value.as_array()) {
        if !items
            .iter()
            .all(|item| output_schema_matches(root, item_schema, item))
        {
            return false;
        }
    }
    for keyword in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(keyword).and_then(Value::as_array) {
            let matches = variants
                .iter()
                .filter(|variant| output_schema_matches(root, variant, value))
                .count();
            if match keyword {
                "allOf" => matches != variants.len(),
                "oneOf" => matches != 1,
                "anyOf" => matches == 0,
                _ => unreachable!(),
            } {
                return false;
            }
        }
    }
    true
}

use serde_json::Value;

// Test-only evaluator for the published output schema's keyword subset. Reject
// unhandled keywords so future schema edits cannot silently weaken fixture checks.
pub fn output_schema_matches(root: &Value, schema: &Value, value: &Value) -> bool {
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

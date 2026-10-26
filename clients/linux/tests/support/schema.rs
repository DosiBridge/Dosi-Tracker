//! A small JSON Schema validator for the schemas in `tests/schemas/`: the
//! subset they use (`type`, `required`, `properties`, `additionalProperties`,
//! `items`, `enum`, `const`, `minimum`, `maximum`, `anyOf`, `$ref` to
//! `#/$defs/...`). Unknown keywords are refused, so a schema cannot claim
//! more than this checks.

use std::path::Path;

use serde_json::Value;

const KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
    "$ref",
    "title",
    "description",
    "type",
    "required",
    "properties",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minimum",
    "maximum",
    "anyOf",
];

/// Load `tests/schemas/<name>`.
pub fn load(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("schemas")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// Validate `value` against `schema`; the errors name the JSON path.
pub fn validate(schema: &Value, value: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Panic with every error when `value` does not match `schema`.
pub fn assert_valid(schema: &Value, value: &Value) {
    if let Err(errors) = validate(schema, value) {
        panic!(
            "schema validation failed:\n  {}\nvalue: {value:#}",
            errors.join("\n  ")
        );
    }
}

fn type_matches(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        other => panic!("unknown schema type {other}"),
    }
}

fn check(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        if schema == &Value::Bool(false) {
            errors.push(format!("{path}: not allowed"));
        }
        return;
    };
    for key in schema.keys() {
        assert!(
            KEYWORDS.contains(&key.as_str()),
            "unsupported schema keyword {key}"
        );
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference
            .strip_prefix("#/$defs/")
            .unwrap_or_else(|| panic!("unsupported $ref {reference}"));
        let target = &root["$defs"][name];
        assert!(!target.is_null(), "missing $defs/{name}");
        check(root, target, value, path, errors);
    }
    if let Some(types) = schema.get("type") {
        let names: Vec<&str> = match types {
            Value::String(name) => vec![name.as_str()],
            Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
            _ => panic!("bad type keyword"),
        };
        if !names.iter().any(|name| type_matches(name, value)) {
            errors.push(format!("{path}: expected {names:?}, got {value}"));
            return;
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            errors.push(format!("{path}: {value} is not one of {options:?}"));
        }
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            errors.push(format!("{path}: expected {expected}, got {value}"));
        }
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if n < min {
                errors.push(format!("{path}: {n} < {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if n > max {
                errors.push(format!("{path}: {n} > {max}"));
            }
        }
    }
    if let Some(any) = schema.get("anyOf").and_then(Value::as_array) {
        let matched = any.iter().any(|option| {
            let mut sub = Vec::new();
            check(root, option, value, path, &mut sub);
            sub.is_empty()
        });
        if !matched {
            errors.push(format!("{path}: matches none of anyOf"));
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    errors.push(format!("{path}: missing {key}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, item) in object {
            let item_path = format!("{path}.{key}");
            match properties.and_then(|p| p.get(key)) {
                Some(sub) => check(root, sub, item, &item_path, errors),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        errors.push(format!("{item_path}: unexpected property"))
                    }
                    Some(sub @ Value::Object(_)) => check(root, sub, item, &item_path, errors),
                    _ => {}
                },
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            check(root, items, item, &format!("{path}[{i}]"), errors);
        }
    }
}

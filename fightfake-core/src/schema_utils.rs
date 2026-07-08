use std::path::Path;

use anyhow::{bail, Context, Result};
use jsonschema::JSONSchema;
use serde_json::Value;

/// Validate `payload_path` (a JSON file) against the JSON Schema at `schema_path`.
pub fn validate_json_against_schema(
    payload_path: &Path,
    schema_path: &Path,
    schema_label: &str,
) -> Result<()> {
    let payload = read_json(payload_path)
        .with_context(|| format!("failed to load payload for schema {schema_label}"))?;
    let schema = read_json(schema_path)
        .with_context(|| format!("failed to load schema {}", schema_path.display()))?;

    let compiled = JSONSchema::compile(&schema)
        .map_err(|e| anyhow::anyhow!("failed to compile schema {}: {e}", schema_path.display()))?;
    if let Err(errors) = compiled.validate(&payload) {
        let details = errors
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "payload {} failed {schema_label} validation:\n{details}",
            payload_path.display(),
        );
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read JSON file {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON in {}", path.display()))
}

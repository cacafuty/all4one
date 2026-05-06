pub mod schema;

use crate::config::schema::Config;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::sync::Arc;
use toml::Value;

pub fn load(path: &str) -> Result<Arc<Config>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("cannot read config file: {path}"))?;

    // Pre-validate required fields for clearer startup errors.
    let value: Value = raw
        .parse::<Value>()
        .map_err(|e| anyhow!("invalid TOML in {path}: {e}"))?;

    ensure_required(&value, "node.tier")?;
    ensure_required(&value, "node.availability")?;
    ensure_required(&value, "node.data_dir")?;
    ensure_required(&value, "security.mode")?;

    let parsed: Config =
        toml::from_str(&raw).map_err(|e| anyhow!("invalid config schema in {path}: {e}"))?;

    if parsed.node.tier > 2 {
        return Err(anyhow!("node.tier must be 0, 1, or 2"));
    }

    if parsed.security.mode != "dev" && parsed.security.mode != "prod" {
        return Err(anyhow!("security.mode must be 'dev' or 'prod'"));
    }

    Ok(Arc::new(parsed))
}

fn ensure_required(value: &Value, key_path: &str) -> Result<()> {
    let mut cursor = value;
    for part in key_path.split('.') {
        cursor = cursor
            .get(part)
            .ok_or_else(|| anyhow!("{key_path} is required"))?;
    }

    if cursor.is_str() && cursor.as_str().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("{key_path} is required"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_required_field() {
        let toml = r#"
[node]
availability = "always"
data_dir = "/tmp/all4one"

[roles]
scheduler = true
executor = true

[network]

[discovery]

[security]
mode = "dev"

[executor]

[capabilities]

[logging]
"#;

        let value: Value = toml.parse::<Value>().expect("valid toml value");
        let err = ensure_required(&value, "node.tier").expect_err("must fail missing tier");
        assert!(err.to_string().contains("node.tier is required"));
    }
}

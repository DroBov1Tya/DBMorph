use std::collections::HashMap;

use anyhow::{Result, bail};

// Deduplicates and sanitizes column names: a blank name becomes cN, collisions get a
// numeric suffix so downstream schemas always have unique field names.
pub fn unique_column_names(headers: &[String]) -> Vec<String> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut names = Vec::with_capacity(headers.len());

    for (i, header) in headers.iter().enumerate() {
        let base = if header.trim().is_empty() {
            format!("c{i}")
        } else {
            header.trim().to_string()
        };

        let counter = seen.entry(base.clone()).or_insert(0);
        let name = if *counter == 0 {
            base.clone()
        } else {
            format!("{base}_{counter}")
        };
        *counter += 1;
        names.push(name);
    }

    names
}

// Parses a --columns "a,b,c" spec into a name list.
pub fn parse_override(spec: &str) -> Vec<String> {
    spec.split(',').map(|s| s.trim().to_string()).collect()
}

// Resolves the output column names: the manual --columns list when given
// (its count must match the source), otherwise the deduplicated auto names.
pub fn resolve_output_names(auto: &[String], override_spec: Option<&str>) -> Result<Vec<String>> {
    match override_spec {
        None => Ok(unique_column_names(auto)),
        Some(spec) => {
            let manual = parse_override(spec);
            if manual.len() != auto.len() {
                bail!(
                    "--columns has {} name(s) but the source has {} column(s)",
                    manual.len(),
                    auto.len()
                );
            }
            if manual.iter().any(|n| n.is_empty()) {
                bail!("--columns contains an empty name");
            }
            Ok(unique_column_names(&manual))
        }
    }
}

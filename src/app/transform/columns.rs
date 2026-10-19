use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

// Deduplicates and sanitizes column names: a blank name becomes cN, a repeated
// name gets the first free numeric suffix.
pub fn unique_column_names(headers: &[String]) -> Vec<String> {
    dedup_names(headers, str::to_string)
}

// Same as unique_column_names, but names that differ only in ASCII case count
// as repeats: SQLite treats "Id" and "id" as the same column.
pub fn unique_column_names_nocase(headers: &[String]) -> Vec<String> {
    dedup_names(headers, str::to_ascii_lowercase)
}

fn dedup_names(headers: &[String], key: fn(&str) -> String) -> Vec<String> {
    let mut taken: HashSet<String> = HashSet::with_capacity(headers.len());
    let mut next_suffix: HashMap<String, u32> = HashMap::new();
    let mut names = Vec::with_capacity(headers.len());

    for (i, header) in headers.iter().enumerate() {
        let trimmed = header.trim();
        let base = if trimmed.is_empty() {
            format!("c{i}")
        } else {
            trimmed.to_string()
        };

        let mut name = base.clone();
        if !taken.insert(key(&name)) {
            let suffix = next_suffix.entry(key(&base)).or_insert(1);
            loop {
                name = format!("{base}_{suffix}");
                *suffix += 1;
                if taken.insert(key(&name)) {
                    break;
                }
            }
        }
        names.push(name);
    }

    names
}

// Quotes an SQL identifier (table or column name) so any text is safe in a
// statement: wrapped in double quotes, inner quotes doubled.
pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(raw: &[&str]) -> Vec<String> {
        let headers: Vec<String> = raw.iter().map(|s| s.to_string()).collect();
        unique_column_names(&headers)
    }

    #[test]
    fn blank_names_become_positional() {
        assert_eq!(names(&["a", " ", ""]), ["a", "c1", "c2"]);
    }

    #[test]
    fn repeats_get_numeric_suffixes() {
        assert_eq!(names(&["a", "a", "a"]), ["a", "a_1", "a_2"]);
    }

    #[test]
    fn suffixes_skip_names_already_taken() {
        assert_eq!(names(&["a", "a_1", "a"]), ["a", "a_1", "a_2"]);
        assert_eq!(names(&["c1", ""]), ["c1", "c1_1"]);
    }

    #[test]
    fn case_matters_unless_folding_is_asked_for() {
        let headers: Vec<String> = ["Id", "id", "ID"].iter().map(|s| s.to_string()).collect();
        assert_eq!(unique_column_names(&headers), ["Id", "id", "ID"]);
        assert_eq!(unique_column_names_nocase(&headers), ["Id", "id_1", "ID_2"]);
    }

    #[test]
    fn nocase_output_is_always_unique_ignoring_case() {
        let headers: Vec<String> = ["x", "X_1", "x", "x_1", "", "c4", "X"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = unique_column_names_nocase(&headers);
        let lowered: HashSet<String> = out.iter().map(|n| n.to_ascii_lowercase()).collect();
        assert_eq!(lowered.len(), out.len(), "{out:?}");
    }

    #[test]
    fn quote_identifier_escapes_inner_quotes() {
        assert_eq!(quote_identifier("users"), "\"users\"");
        assert_eq!(
            quote_identifier("a\"; DROP TABLE t; --"),
            "\"a\"\"; DROP TABLE t; --\""
        );
    }

    #[test]
    fn override_must_match_source_width() {
        let auto = vec!["a".to_string(), "b".to_string()];
        assert!(resolve_output_names(&auto, Some("x")).is_err());
        assert!(resolve_output_names(&auto, Some("x,")).is_err());
        assert_eq!(
            resolve_output_names(&auto, Some("x, y")).unwrap(),
            ["x", "y"]
        );
    }
}

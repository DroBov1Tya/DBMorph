use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Deserializer, Value};

use crate::config;

enum JsonShape {
    /// A single top-level `[ {...}, {...} ]` array.
    Array,
    /// One or more whitespace/newline separated objects (NDJSON or concatenated).
    Stream,
}

fn detect_shape<P: AsRef<Path>>(path: P) -> Result<JsonShape> {
    let path = path.as_ref();
    let mut reader =
        BufReader::new(File::open(path).with_context(|| format!("failed to open JSON {path:?}"))?);

    let mut byte = [0u8; 1];
    loop {
        let n = reader.read(&mut byte)?;
        if n == 0 {
            bail!("empty JSON input");
        }
        if byte[0].is_ascii_whitespace() {
            continue;
        }
        return Ok(if byte[0] == b'[' {
            JsonShape::Array
        } else {
            JsonShape::Stream
        });
    }
}

fn iter_values<P: AsRef<Path>>(path: P) -> Result<Box<dyn Iterator<Item = Result<Value>>>> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open JSON {path:?}"))?;
    let reader = BufReader::with_capacity(config::READ_BUFFER_BYTES, file);

    match detect_shape(path)? {
        JsonShape::Array => {
            let values: Vec<Value> =
                serde_json::from_reader(reader).context("failed to parse JSON array")?;
            Ok(Box::new(values.into_iter().map(Ok)))
        }
        JsonShape::Stream => {
            let stream = Deserializer::from_reader(reader).into_iter::<Value>();
            Ok(Box::new(stream.map(|r| r.map_err(anyhow::Error::from))))
        }
    }
}

/// Column schema = union of object keys across the whole input, in the order
/// each key first appears. Preserves the original JSON field names.
pub fn json_schema<P: AsRef<Path>>(path: P) -> Result<Vec<String>> {
    let mut columns: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for value in iter_values(path)? {
        if let Value::Object(map) = value? {
            for key in map.keys() {
                if seen.insert(key.clone()) {
                    columns.push(key.clone());
                }
            }
        }
    }

    if columns.is_empty() {
        bail!("no JSON objects with keys found");
    }
    Ok(columns)
}

/// Streams rows aligned to `columns`: missing keys become empty, scalars are
/// stringified, nested values re-serialized as compact JSON.
pub fn json_row_reader<P: AsRef<Path>>(
    path: P,
    columns: Vec<String>,
) -> Result<impl Iterator<Item = Result<Vec<String>>>> {
    let values = iter_values(path)?;

    let iter = values.map(move |value| {
        value.and_then(|value| match value {
            Value::Object(map) => Ok(columns
                .iter()
                .map(|col| map.get(col).map(value_to_string).unwrap_or_default())
                .collect::<Vec<String>>()),
            other => bail!("expected a JSON object, got {}", kind_of(&other)),
        })
    });

    Ok(iter)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

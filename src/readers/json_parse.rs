use colored::*;
use futures::stream::{self, Stream};
use futures::StreamExt;
use serde_json::Value;
use tracing::error;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

fn flatten_json(value: &Value, prefix: String, out: &mut HashMap<String, String>) {
    // Recursively flattens a nested JSON structure into a flat map with dot-separated keys.
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let new_prefix = if prefix.is_empty() {
                    k.to_string()
                } else {
                    format!("{}.{}", prefix, k)
                };
                flatten_json(v, new_prefix, out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let new_prefix = format!("{}.{}", prefix, i);
                flatten_json(v, new_prefix, out);
            }
        }
        Value::Null => {
            out.insert(prefix, "null".to_string());
        }
        other => {
            out.insert(prefix, other.to_string());
        }
    }
}

pub async fn read_json_lines_flat<P: AsRef<Path>>(
    path: P,
) -> impl Stream<Item = Result<BTreeMap<String, String>, String>> {
    // Reads a JSON Lines file asynchronously, flattens each JSON object, and returns them as a stream of ordered maps.
    let file = match File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            return stream::once(async move { Err(format!("Failed to open file: {}", e)) }).boxed()
        }
    };

    let reader = BufReader::new(file);
    let lines = reader.lines();

    stream::unfold(lines, |mut lines| async {
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => match serde_json::from_str::<Value>(&line) {
                    Ok(json) => {
                        let mut map = HashMap::new();
                        flatten_json(&json, "".to_string(), &mut map);
                        let ordered_map: BTreeMap<_, _> = map.into_iter().collect();
                        return Some((Ok(ordered_map), lines));
                    }
                    Err(e) => {
                        return Some((Err(format!("JSON parse error: {}", e)), lines));
                    }
                },
                Ok(None) => return None,
                Err(e) => return Some((Err(format!("I/O error: {}", e)), lines)),
            }
        }
    })
    .boxed()
}

pub async fn count_keys_in_first_json<P: AsRef<std::path::Path>>(path: P) -> Result<i32, String> {
    // Reads the first JSON line from a file, flattens it, and returns the number of keys found.
    let mut stream = read_json_lines_flat(path).await;

    if let Some(result) = stream.next().await {
        match result {
            Ok(map) => Ok(map.len() as i32),
            Err(e) => Err(format!("Error reading first JSON line: {}", e)),
        }
    } else {
        Err("No lines found in file".to_string())
    }
}

pub async fn count_objects_with_keys<P: AsRef<Path>>(path: P) -> Result<u32, String> {
    // Counts the number of non-empty flattened JSON objects in a JSON Lines file.
    let mut stream = read_json_lines_flat(path).await;
    let mut count = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(map) if !map.is_empty() => count += 1,
            Ok(_) => {}
            Err(err) => error!("{} Error: {}", "🚫 [ERROR]".red().bold(), err),
        }
    }

    Ok(count)
}

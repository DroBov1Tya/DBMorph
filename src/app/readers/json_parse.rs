use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Deserializer, Value};

use crate::config;

enum JsonShape {
    // A single top-level array of objects.
    Array,
    // One or more whitespace/newline separated objects (NDJSON or concatenated).
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

// Streams top-level elements of a JSON array without materializing the file.
// A byte scanner tracks brace/bracket depth and string state, emitting one
// element at a time so peak memory stays at a single element plus the buffer.
struct ArrayReader {
    reader: BufReader<File>,
    started: bool,
    first: bool,
    done: bool,
}

impl ArrayReader {
    fn new(reader: BufReader<File>) -> Self {
        Self {
            reader,
            started: false,
            first: true,
            done: false,
        }
    }

    fn peek_sig(&mut self) -> Result<Option<u8>> {
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                return Ok(None);
            }
            let mut ws = 0;
            while ws < chunk.len() && chunk[ws].is_ascii_whitespace() {
                ws += 1;
            }
            if ws > 0 {
                self.reader.consume(ws);
                continue;
            }
            return Ok(Some(chunk[0]));
        }
    }

    fn consume_one(&mut self) -> Result<()> {
        let chunk = self.reader.fill_buf()?;
        if !chunk.is_empty() {
            self.reader.consume(1);
        }
        Ok(())
    }

    fn read_structural(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut esc = false;
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                bail!("unexpected EOF inside JSON value");
            }
            let mut used = chunk.len();
            let mut finished = false;
            for (i, &b) in chunk.iter().enumerate() {
                if in_str {
                    if esc {
                        esc = false;
                    } else if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                    continue;
                }
                match b {
                    b'"' => in_str = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            used = i + 1;
                            finished = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            buf.extend_from_slice(&chunk[..used]);
            self.reader.consume(used);
            if finished {
                return Ok(());
            }
        }
    }

    fn read_string(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        let mut esc = false;
        let mut opened = false;
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                bail!("unexpected EOF inside JSON string");
            }
            let mut used = chunk.len();
            let mut finished = false;
            for (i, &b) in chunk.iter().enumerate() {
                if !opened {
                    opened = true;
                    continue;
                }
                if esc {
                    esc = false;
                } else if b == b'\\' {
                    esc = true;
                } else if b == b'"' {
                    used = i + 1;
                    finished = true;
                    break;
                }
            }
            buf.extend_from_slice(&chunk[..used]);
            self.reader.consume(used);
            if finished {
                return Ok(());
            }
        }
    }

    fn read_scalar(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                return Ok(());
            }
            let mut stop_at = None;
            for (i, &b) in chunk.iter().enumerate() {
                if b == b',' || b == b']' || b == b'}' || b.is_ascii_whitespace() {
                    stop_at = Some(i);
                    break;
                }
            }
            match stop_at {
                Some(i) => {
                    buf.extend_from_slice(&chunk[..i]);
                    self.reader.consume(i);
                    return Ok(());
                }
                None => {
                    let len = chunk.len();
                    buf.extend_from_slice(chunk);
                    self.reader.consume(len);
                }
            }
        }
    }

    fn read_value(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        match self.peek_sig()? {
            Some(b'{') | Some(b'[') => self.read_structural(buf),
            Some(b'"') => self.read_string(buf),
            Some(_) => self.read_scalar(buf),
            None => bail!("unexpected EOF, expected a JSON value"),
        }
    }

    fn next_value(&mut self) -> Result<Option<Value>> {
        if self.done {
            return Ok(None);
        }
        if !self.started {
            match self.peek_sig()? {
                Some(b'[') => {
                    self.consume_one()?;
                    self.started = true;
                }
                Some(other) => bail!(
                    "expected '[' at start of JSON array, got {:?}",
                    other as char
                ),
                None => bail!("empty JSON input"),
            }
        }

        match self.peek_sig()? {
            Some(b']') => {
                self.consume_one()?;
                self.done = true;
                return Ok(None);
            }
            Some(b',') => {
                if self.first {
                    bail!("unexpected ',' before first array element");
                }
                self.consume_one()?;
                if let Some(b']') = self.peek_sig()? {
                    self.consume_one()?;
                    self.done = true;
                    return Ok(None);
                }
            }
            Some(_) => {
                if !self.first {
                    bail!("expected ',' or ']' between array elements");
                }
            }
            None => bail!("unexpected EOF in JSON array (missing ']')"),
        }

        self.first = false;
        let mut buf = Vec::new();
        self.read_value(&mut buf)?;
        let value: Value =
            serde_json::from_slice(&buf).context("failed to parse JSON array element")?;
        Ok(Some(value))
    }
}

impl Iterator for ArrayReader {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_value() {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

fn iter_values<P: AsRef<Path>>(path: P) -> Result<Box<dyn Iterator<Item = Result<Value>>>> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open JSON {path:?}"))?;
    let reader = BufReader::with_capacity(config::READ_BUFFER_BYTES, file);

    match detect_shape(path)? {
        JsonShape::Array => Ok(Box::new(ArrayReader::new(reader))),
        JsonShape::Stream => {
            let stream = Deserializer::from_reader(reader).into_iter::<Value>();
            Ok(Box::new(stream.map(|r| r.map_err(anyhow::Error::from))))
        }
    }
}

// Column schema is the union of object keys across the whole input, in the order
// each key first appears. Preserves the original JSON field names.
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

// Streams rows aligned to columns: missing keys become empty, scalars are
// stringified, nested values re-serialized as compact JSON.
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

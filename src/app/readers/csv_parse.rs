use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use csv::ReaderBuilder;
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;

use crate::config;

// Read adapter that repairs one physical line at a time: a line whose double
// quotes are unbalanced (an unterminated quote that would otherwise make the
// parser swallow following lines) gets a closing quote appended before its
// newline, keeping every physical line a separate record.
struct RepairReader<R: Read> {
    inner: BufReader<R>,
    buf: Vec<u8>,
    pos: usize,
    repairs: Arc<AtomicU64>,
    done: bool,
}

impl<R: Read> RepairReader<R> {
    fn new(inner: R, repairs: Arc<AtomicU64>) -> Self {
        Self {
            inner: BufReader::with_capacity(config::READ_BUFFER_BYTES, inner),
            buf: Vec::new(),
            pos: 0,
            repairs,
            done: false,
        }
    }

    fn fill_line(&mut self) -> std::io::Result<bool> {
        self.buf.clear();
        self.pos = 0;
        if self.inner.read_until(b'\n', &mut self.buf)? == 0 {
            return Ok(false);
        }

        let quotes = self.buf.iter().filter(|&&b| b == b'"').count();
        if quotes % 2 == 1 {
            let mut end = self.buf.len();
            if end > 0 && self.buf[end - 1] == b'\n' {
                end -= 1;
                if end > 0 && self.buf[end - 1] == b'\r' {
                    end -= 1;
                }
            }
            self.buf.insert(end, b'"');
            self.repairs.fetch_add(1, Ordering::Relaxed);
        }
        Ok(true)
    }
}

impl<R: Read> Read for RepairReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.pos < self.buf.len() {
                let n = (self.buf.len() - self.pos).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done || !self.fill_line()? {
                self.done = true;
                return Ok(0);
            }
        }
    }
}

fn decoded_source(path: &Path, encoding: &str) -> Result<impl Read + use<>> {
    let file = File::open(path).with_context(|| format!("failed to open CSV {path:?}"))?;
    let rust_encoding = Encoding::for_label(encoding.as_bytes())
        .with_context(|| format!("unsupported encoding: {encoding}"))?;
    Ok(DecodeReaderBytesBuilder::new()
        .encoding(Some(rust_encoding))
        .build(BufReader::with_capacity(config::READ_BUFFER_BYTES, file)))
}

fn wrap_repair<R: Read + 'static>(
    reader: R,
    repair: bool,
    repairs: Arc<AtomicU64>,
) -> Box<dyn Read> {
    if repair {
        Box::new(RepairReader::new(reader, repairs))
    } else {
        Box::new(reader)
    }
}

// Loads the repair count and warns when any lines were repaired.
pub fn report_repairs(repairs: &Arc<AtomicU64>) -> u64 {
    let n = repairs.load(Ordering::Relaxed);
    if n > 0 {
        crate::app::utils::ui::warn(&format!(
            "repaired {n} row(s): closed unterminated quote at line end"
        ));
    }
    n
}

// Streams CSV rows. When repair is set, unterminated quotes are closed at
// line ends so a stray quote cannot merge rows; the returned counter reports
// how many lines were repaired.
pub async fn csv_row_reader<P: AsRef<Path>>(
    csv_file: P,
    delimiter: u8,
    encoding: &str,
    has_headers: bool,
    quoting: bool,
    repair: bool,
) -> Result<(impl Iterator<Item = Result<Vec<String>>>, Arc<AtomicU64>)> {
    let repairs = Arc::new(AtomicU64::new(0));
    let source = wrap_repair(
        decoded_source(csv_file.as_ref(), encoding)?,
        repair && quoting,
        repairs.clone(),
    );

    let csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(has_headers)
        .flexible(true)
        .quoting(quoting)
        .from_reader(source);

    let iter = csv_reader.into_records().map(|res| {
        res.map(|record| {
            record
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
        })
        .map_err(anyhow::Error::from)
    });

    Ok((iter, repairs))
}

// Column names: header row when present, else generated names c0, c1 and so on.
pub async fn csv_headers<P: AsRef<Path>>(
    csv_file: P,
    delimiter: u8,
    encoding: &str,
    has_headers: bool,
    quoting: bool,
    repair: bool,
) -> Result<Vec<String>> {
    let source = wrap_repair(
        decoded_source(csv_file.as_ref(), encoding)?,
        repair && quoting,
        Arc::new(AtomicU64::new(0)),
    );

    let mut csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .quoting(quoting)
        .from_reader(source);

    let mut first = csv::StringRecord::new();
    if !csv_reader.read_record(&mut first)? {
        bail!("CSV file has no rows");
    }

    if has_headers {
        Ok(first.iter().map(|s| s.to_string()).collect())
    } else {
        Ok((0..first.len()).map(|i| format!("c{i}")).collect())
    }
}

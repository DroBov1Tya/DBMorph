use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use csv::ReaderBuilder;
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;

use crate::app::transform::{self, repair};
use crate::app::utils::progress::Progress;
use crate::app::utils::ui;
use crate::args::AppArgs;
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

// How a delimited text source is read.
pub struct CsvOptions {
    pub delimiter: u8,
    pub encoding: String,
    // Lines that are not valid UTF-8 are decoded with this instead.
    pub fallback: Option<String>,
    pub has_headers: bool,
    pub quoting: bool,
    // Close an unterminated quote at the line end (only when quoting).
    pub repair: bool,
}

impl CsvOptions {
    pub fn from_args(args: &AppArgs, encoding: String) -> Self {
        Self {
            delimiter: args.delimiter,
            encoding,
            fallback: args.fallback_encoding.clone(),
            has_headers: !args.no_header,
            quoting: !args.no_quote,
            repair: !args.no_repair,
        }
    }
}

// Read adapter for UTF-8 files with legacy-encoded parts: each line that is
// not valid UTF-8 is decoded with the fallback encoding instead of turning
// its bytes into U+FFFD.
struct FallbackDecoder<R: Read> {
    inner: BufReader<R>,
    fallback: &'static Encoding,
    raw: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
    first: bool,
    decoded: Arc<AtomicU64>,
}

impl<R: Read> FallbackDecoder<R> {
    fn fill_line(&mut self) -> std::io::Result<bool> {
        self.raw.clear();
        self.out.clear();
        self.pos = 0;
        if self.inner.read_until(b'\n', &mut self.raw)? == 0 {
            return Ok(false);
        }

        let mut line = &self.raw[..];
        if std::mem::take(&mut self.first) {
            line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
        }
        match std::str::from_utf8(line) {
            Ok(text) => self.out.extend_from_slice(text.as_bytes()),
            Err(_) => {
                let (text, _) = self.fallback.decode_without_bom_handling(line);
                self.out.extend_from_slice(text.as_bytes());
                self.decoded.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(true)
    }
}

impl<R: Read> Read for FallbackDecoder<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.out.len() {
            if !self.fill_line()? {
                return Ok(0);
            }
        }
        let n = (self.out.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn decoded_source(
    path: &Path,
    opts: &CsvOptions,
    progress: &Progress,
    fallback_lines: Arc<AtomicU64>,
) -> Result<Box<dyn Read + Send>> {
    let file = File::open(path).with_context(|| format!("failed to open CSV {path:?}"))?;
    let file = BufReader::with_capacity(config::READ_BUFFER_BYTES, progress.reader(file));
    let encoding = Encoding::for_label(opts.encoding.as_bytes())
        .with_context(|| format!("unsupported encoding: {}", opts.encoding))?;

    let fallback = opts
        .fallback
        .as_deref()
        .and_then(transform::encoding::by_label);
    if let (true, Some(fallback)) = (encoding == encoding_rs::UTF_8, fallback) {
        return Ok(Box::new(FallbackDecoder {
            inner: file,
            fallback,
            raw: Vec::new(),
            out: Vec::new(),
            pos: 0,
            first: true,
            decoded: fallback_lines,
        }));
    }

    Ok(Box::new(
        DecodeReaderBytesBuilder::new()
            .encoding(Some(encoding))
            .build(file),
    ))
}

fn wrap_repair<R: Read + Send + 'static>(
    reader: R,
    repair: bool,
    repairs: Arc<AtomicU64>,
) -> Box<dyn Read + Send> {
    if repair {
        Box::new(RepairReader::new(reader, repairs))
    } else {
        Box::new(reader)
    }
}

fn csv_reader<R: Read>(source: R, opts: &CsvOptions, has_headers: bool) -> csv::Reader<R> {
    ReaderBuilder::new()
        .delimiter(opts.delimiter)
        .has_headers(has_headers)
        .flexible(true)
        .quoting(opts.quoting)
        .from_reader(source)
}

fn to_strings(record: &csv::StringRecord) -> Vec<String> {
    record.iter().map(|s| s.to_string()).collect()
}

// Counters of every repair made while reading a csv/txt source.
#[derive(Clone, Default)]
pub struct CsvStats {
    quotes: Arc<AtomicU64>,
    fallback_lines: Arc<AtomicU64>,
    fallback: Option<String>,
    rows: repair::RepairCounters,
}

impl CsvStats {
    // Prints what was repaired and returns the count for the exit code. A
    // dropped trailing delimiter is the file's format, not damage.
    pub fn finish(&self) -> u64 {
        let quotes = self.quotes.load(Ordering::Relaxed);
        if quotes > 0 {
            ui::warn(&format!(
                "repaired {quotes} row(s): closed unterminated quote at line end"
            ));
        }
        let fallback = self.fallback_lines.load(Ordering::Relaxed);
        if fallback > 0 {
            ui::warn(&format!(
                "decoded {fallback} line(s) as {} (not valid UTF-8)",
                self.fallback.as_deref().unwrap_or("fallback")
            ));
        }
        let joined = self.rows.joined();
        if joined > 0 {
            ui::warn(&format!("rejoined {joined} line break(s) inside fields"));
        }
        let realigned = self.rows.realigned();
        if realigned > 0 {
            ui::warn(&format!(
                "realigned {realigned} row(s) shifted by a delimiter inside a field"
            ));
        }
        let trailing = self.rows.trailing();
        if trailing > 0 {
            ui::info(&format!(
                "dropped the trailing delimiter on {trailing} row(s)"
            ));
        }
        quotes + fallback + joined + realigned
    }
}

// A csv/txt source ready to stream: source column names, the repaired rows
// and the counters of what was repaired.
pub struct CsvSource {
    pub headers: Vec<String>,
    pub rows: Box<dyn Iterator<Item = Result<Vec<String>>> + Send>,
    pub stats: CsvStats,
}

// Opens a csv/txt source as the command line asks: encoding (detected when
// not given), header, quoting and the opt-in row repairs.
pub async fn open_csv(args: &AppArgs, progress: &Progress) -> Result<CsvSource> {
    let encoding = match &args.encoding {
        Some(enc) => enc.clone(),
        None => transform::encoding::detect_encoding(&args.input_path)?,
    };
    ui::field("charset", &encoding);
    let opts = CsvOptions::from_args(args, encoding);

    let mut headers = csv_headers(&args.input_path, &opts).await?;
    if args.trailing_delimiter && headers.len() > 1 && headers.last().is_some_and(String::is_empty)
    {
        headers.pop();
    }
    let width = headers.len();

    let stats = CsvStats {
        fallback: opts.fallback.clone(),
        ..CsvStats::default()
    };
    let raw = csv_row_reader(&args.input_path, &opts, progress, &stats)?;

    let mut rows: Box<dyn Iterator<Item = Result<Vec<String>>> + Send> = Box::new(raw);
    if args.trailing_delimiter {
        rows = Box::new(repair::drop_trailing_empty(
            rows,
            width,
            stats.rows.trailing.clone(),
        ));
    }
    if args.join_lines {
        rows = Box::new(repair::join_broken_lines(
            rows,
            width,
            stats.rows.joined.clone(),
        ));
    }
    if args.realign {
        rows = Box::new(repair::realign_shifted(
            rows,
            width,
            args.delimiter,
            stats.rows.realigned.clone(),
        ));
    }

    Ok(CsvSource {
        headers,
        rows,
        stats,
    })
}

// Streams CSV rows. When repair is set, unterminated quotes are closed at
// line ends so a stray quote cannot merge rows.
fn csv_row_reader(
    path: &str,
    opts: &CsvOptions,
    progress: &Progress,
    stats: &CsvStats,
) -> Result<impl Iterator<Item = Result<Vec<String>>> + Send + use<>> {
    let source = wrap_repair(
        decoded_source(
            Path::new(path),
            opts,
            progress,
            stats.fallback_lines.clone(),
        )?,
        opts.repair && opts.quoting,
        stats.quotes.clone(),
    );

    let iter = csv_reader(source, opts, opts.has_headers)
        .into_records()
        .map(|res| {
            res.map(|record| to_strings(&record))
                .map_err(anyhow::Error::from)
        });
    Ok(iter)
}

// Column names: header row when present, else generated names c0, c1 and so on.
async fn csv_headers(path: &str, opts: &CsvOptions) -> Result<Vec<String>> {
    let source = wrap_repair(
        decoded_source(
            Path::new(path),
            opts,
            &Progress::unknown(),
            Arc::new(AtomicU64::new(0)),
        )?,
        opts.repair && opts.quoting,
        Arc::new(AtomicU64::new(0)),
    );

    let mut reader = csv_reader(source, opts, false);
    let mut first = csv::StringRecord::new();
    if !reader.read_record(&mut first)? {
        bail!("CSV file has no rows");
    }

    if opts.has_headers {
        Ok(to_strings(&first))
    } else {
        Ok((0..first.len()).map(|i| format!("c{i}")).collect())
    }
}

// Parses already decoded sample text the way the real read would, header row
// included, for the doctor's checks.
pub fn parse_sample(text: &str, opts: &CsvOptions, max_records: usize) -> Vec<Vec<String>> {
    let source = wrap_repair(
        std::io::Cursor::new(text.as_bytes().to_vec()),
        opts.repair && opts.quoting,
        Arc::new(AtomicU64::new(0)),
    );
    csv_reader(source, opts, false)
        .into_records()
        .take(max_records)
        .filter_map(|r| r.ok())
        .map(|r| to_strings(&r))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(bytes: &[u8], fallback: &'static Encoding) -> (String, u64) {
        let decoded = Arc::new(AtomicU64::new(0));
        let mut reader = FallbackDecoder {
            inner: BufReader::new(bytes),
            fallback,
            raw: Vec::new(),
            out: Vec::new(),
            pos: 0,
            first: true,
            decoded: decoded.clone(),
        };
        let mut out = String::new();
        reader.read_to_string(&mut out).unwrap();
        (out, decoded.load(Ordering::Relaxed))
    }

    #[test]
    fn fallback_decodes_only_the_lines_that_are_not_utf8() {
        let mut bytes = "\u{feff}id,name\n1,Привет\n".as_bytes().to_vec();
        bytes.extend_from_slice(b"2,");
        bytes.extend_from_slice(&encoding_rs::WINDOWS_1251.encode("Мир").0);
        bytes.extend_from_slice(b"\n3,ok");
        let (text, fixed) = decode_all(&bytes, encoding_rs::WINDOWS_1251);
        assert_eq!(text, "id,name\n1,Привет\n2,Мир\n3,ok");
        assert_eq!(fixed, 1);
    }

    #[test]
    fn sample_parsing_keeps_the_header_and_repairs_quotes() {
        let opts = CsvOptions {
            delimiter: b',',
            encoding: "utf-8".into(),
            fallback: None,
            has_headers: true,
            quoting: true,
            repair: true,
        };
        let rows = parse_sample("a,b\n1,\"x\n2,y\n", &opts, 10);
        assert_eq!(rows, vec![vec!["a", "b"], vec!["1", "x"], vec!["2", "y"]]);
    }
}

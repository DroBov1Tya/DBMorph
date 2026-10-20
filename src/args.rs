use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "DBMorph",
    version,
    author = "DroBoV1tya",
    about = "Convert data files between formats and databases."
)]
pub struct AppArgs {
    /// Input file path
    #[arg(short = 'i', long = "input", value_name = "FILE")]
    pub input_path: String,

    /// Output file path (extension added automatically)
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output_path: String,

    /// Source format: csv, txt, json, xlsx, parquet, sqlite, sql
    #[arg(
        short = 'f',
        long = "from",
        value_name = "FORMAT",
        default_value = "csv"
    )]
    pub input_file_type: String,

    /// Target format: sqlite, parquet, csv
    #[arg(
        short = 't',
        long = "to",
        value_name = "TARGET",
        default_value = "sqlite"
    )]
    pub database_type: String,

    /// Table to read (sqlite source) or create (sqlite target)
    #[arg(
        short = 'n',
        long = "table-name",
        value_name = "NAME",
        default_value = crate::config::DEFAULT_TABLE_NAME
    )]
    pub table_name: String,

    /// Input encoding (auto-detected when omitted)
    #[arg(short = 'e', long = "encoding", value_name = "ENCODING")]
    pub encoding: Option<String>,

    /// Field delimiter for csv/txt input
    #[arg(
        short = 'd',
        long = "delimiter",
        value_name = "CHAR",
        default_value = ",",
        value_parser = parse_delimiter
    )]
    pub delimiter: u8,

    /// Parquet compression codec
    #[arg(
        short = 'c',
        long = "compression",
        value_name = "CODEC",
        value_enum,
        default_value_t = Compression::Zstd
    )]
    pub compression: Compression,

    /// Compression level (zstd 1-22, gzip 0-9, brotli 0-11; codec default when omitted)
    #[arg(short = 'l', long = "level", value_name = "N")]
    pub level: Option<i32>,

    /// Rows buffered per write/insert batch
    #[arg(
        short = 'b',
        long = "batch-size",
        value_name = "ROWS",
        default_value_t = crate::config::DEFAULT_BATCH_SIZE
    )]
    pub batch_size: u32,

    /// Drop the target table before inserting (sqlite target)
    #[arg(short = 'x', long = "drop-existing")]
    pub drop_existing: bool,

    /// Treat the first csv/txt row as data, not column names
    #[arg(short = 'H', long = "no-header")]
    pub no_header: bool,

    /// Truncate every field to N characters (0 = keep full value)
    #[arg(long = "max-field", value_name = "N", default_value_t = 0)]
    pub max_field: usize,

    /// Strip the given characters from BOTH edges of every cell. Each char in
    /// the set is trimmed, e.g. `--trim '"'` turns `"somename"` into
    /// `somename`; `--trim '"'\'' '` also strips single quotes and spaces.
    #[arg(long = "trim", value_name = "CHARS")]
    pub trim: Option<String>,

    /// Abort on the first malformed row instead of skipping it
    #[arg(short = 's', long = "strict")]
    pub strict: bool,

    /// Infer native Parquet column types (int/float/bool) instead of all-text
    #[arg(long = "infer-types")]
    pub infer_types: bool,

    /// SQLite table layout: fts (FTS5, all text, MATCH search) or typed
    /// (STRICT table, INTEGER/REAL/TEXT columns inferred from a sample)
    #[arg(
        long = "sqlite-mode",
        value_name = "MODE",
        value_enum,
        default_value_t = SqliteMode::Fts
    )]
    pub sqlite_mode: SqliteMode,

    /// Rows sampled to infer column types (--infer-types, --sqlite-mode typed).
    /// The sample is held in memory; a value that does not fit the inferred
    /// type later on is stored as NULL and reported (an error with --strict).
    #[arg(
        long = "infer-rows",
        value_name = "ROWS",
        default_value_t = crate::config::INFER_SAMPLE_ROWS,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub infer_rows: u32,

    /// Disable csv/txt quote handling: every line break ends a row
    /// (use for naive dumps where an unclosed quote would merge rows)
    #[arg(long = "no-quote")]
    pub no_quote: bool,

    /// Disable on-the-fly row repair (default on for csv/txt): normally an
    /// unterminated quote is closed at the line end so a stray quote cannot
    /// merge following rows; pass this to keep RFC multi-line quoted fields
    #[arg(long = "no-repair")]
    pub no_repair: bool,

    /// Count rows up front for a progress percentage (sqlite source only).
    /// This is a full table scan and delays the start on huge databases.
    #[arg(long = "count")]
    pub count: bool,

    /// Manually set output column names as a comma-separated list; the count
    /// must match the source columns. Overrides detected/synthetic names.
    #[arg(long = "columns", value_name = "A,B,C")]
    pub columns: Option<String>,

    /// Check the input for damage (encoding, delimiter, header, broken or
    /// shifted rows), print the findings and a suggested command, and exit
    #[arg(long = "doctor")]
    pub doctor: bool,

    /// Report problems but do not apply the safe fixes automatically
    #[arg(long = "no-auto-fix")]
    pub no_auto_fix: bool,

    /// Rejoin rows split by a stray line break inside an unquoted field
    #[arg(long = "join-lines")]
    pub join_lines: bool,

    /// Move fields shifted by an unescaped delimiter back into their column
    /// (needs typed columns to tell where the extra delimiter is)
    #[arg(long = "realign")]
    pub realign: bool,

    /// Drop the empty last field left by a delimiter at the end of each line
    #[arg(long = "trailing-delimiter")]
    pub trailing_delimiter: bool,

    /// Decode lines that are not valid UTF-8 with this encoding instead
    /// (files glued together from UTF-8 and legacy-encoded parts)
    #[arg(long = "fallback-encoding", value_name = "ENCODING", value_parser = parse_single_byte_encoding)]
    pub fallback_encoding: Option<String>,

    /// Repair text that was UTF-8 but got decoded as this single-byte
    /// encoding somewhere upstream (e.g. "Ã©" back to "é" for windows-1252)
    #[arg(long = "fix-mojibake", value_name = "ENCODING", value_parser = parse_single_byte_encoding)]
    pub fix_mojibake: Option<String>,

    /// Ids of the options given on the command line. Auto-fixes never
    /// override what the user set explicitly.
    #[arg(skip)]
    pub explicit: Vec<String>,
}

impl AppArgs {
    pub fn is_explicit(&self, id: &str) -> bool {
        self.explicit.iter().any(|e| e == id)
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Compression {
    Zstd,
    Snappy,
    Gzip,
    Brotli,
    Lz4,
    None,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SqliteMode {
    Fts,
    Typed,
}

fn parse_delimiter(raw: &str) -> Result<u8, String> {
    Ok(match raw {
        "t" | "\\t" => b'\t',
        "n" | "\\n" => b'\n',
        "r" | "\\r" => b'\r',
        "," => b',',
        ";" => b';',
        "|" => b'|',
        other => other.bytes().next().unwrap_or(b','),
    })
}

// Single-byte encodings only: mojibake repair and per-line fallback both map
// one byte to one character.
fn parse_single_byte_encoding(raw: &str) -> Result<String, String> {
    match encoding_rs::Encoding::for_label(raw.as_bytes()) {
        Some(enc) if enc.is_single_byte() => Ok(enc.name().to_lowercase()),
        Some(enc) => Err(format!("{} is not a single-byte encoding", enc.name())),
        None => Err(format!("unknown encoding: {raw}")),
    }
}

pub fn parse_args() -> AppArgs {
    let matches = AppArgs::command().get_matches();
    let mut args = AppArgs::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    args.explicit = matches
        .ids()
        .filter(|id| matches.value_source(id.as_str()) == Some(ValueSource::CommandLine))
        .map(|id| id.to_string())
        .collect();
    args
}

pub fn parse_from<I, T>(argv: I) -> Result<AppArgs, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let matches = AppArgs::command().try_get_matches_from(argv)?;
    let mut args = AppArgs::from_arg_matches(&matches)?;
    args.explicit = matches
        .ids()
        .filter(|id| matches.value_source(id.as_str()) == Some(ValueSource::CommandLine))
        .map(|id| id.to_string())
        .collect();
    Ok(args)
}

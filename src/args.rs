use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
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

    /// Source format: csv, txt, parquet, sql
    #[arg(short = 'f', long = "from", value_name = "FORMAT", default_value = "csv")]
    pub input_file_type: String,

    /// Target format: sqlite, parquet, csv
    #[arg(short = 't', long = "to", value_name = "TARGET", default_value = "sqlite")]
    pub database_type: String,

    /// Destination table name (sqlite target)
    #[arg(short = 'n', long = "table-name", value_name = "NAME", default_value = "main")]
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

pub fn parse_args() -> AppArgs {
    AppArgs::parse()
}

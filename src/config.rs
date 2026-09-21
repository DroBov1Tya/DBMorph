pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_BATCH_SIZE: u32 = 50_000;

/// SQLite bound-parameter ceiling; multi-row INSERTs are split under it.
pub const SQLITE_MAX_PARAMS: usize = 32_766;

/// Negative cache_size (KiB) => pages held in RAM, not on disk.
pub const SQLITE_CACHE_KIB: i64 = 65_536;

/// Sequential read buffer for CSV/text input.
pub const READ_BUFFER_BYTES: usize = 1 << 20;

/// Buffered writer capacity for Parquet output.
pub const WRITE_BUFFER_BYTES: usize = 1 << 20;

/// Batch size for the streaming Parquet record reader.
pub const PARQUET_READ_BATCH: usize = 8_192;

/// Bounded buffer between the SQLite reader task and the writer iterator.
pub const SQLITE_STREAM_CHANNEL: usize = 8_192;

/// Rows sampled to infer column types when `--infer-types` is set.
pub const PARQUET_INFER_SAMPLE: usize = 8_192;

/// A tuple wider than max(expected * FACTOR, MIN) fields is a runaway parse.
pub const SQL_TUPLE_FIELD_FACTOR: usize = 16;
pub const SQL_TUPLE_FIELD_MIN: usize = 4_096;

/// Malformed-row warnings printed before the rest are suppressed.
pub const SKIP_WARN_LIMIT: u64 = 10;

pub const PREVIEW_ROWS: usize = 5;
pub const PREVIEW_COLS: usize = 5;

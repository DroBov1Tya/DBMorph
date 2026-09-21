pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_BATCH_SIZE: u32 = 50_000;

/// Modern SQLite bound-parameter ceiling (SQLITE_MAX_VARIABLE_NUMBER).
/// A multi-row INSERT is split so that rows * columns never exceeds this.
pub const SQLITE_MAX_PARAMS: usize = 32_766;

/// 64 MiB negative cache_size => pages held in RAM, not on disk.
pub const SQLITE_CACHE_KIB: i64 = 65_536;

/// Sequential read buffer for CSV/text input.
pub const READ_BUFFER_BYTES: usize = 1 << 20;

/// Buffered writer capacity for Parquet output.
pub const WRITE_BUFFER_BYTES: usize = 1 << 20;

/// Batch size for the streaming Parquet record reader.
pub const PARQUET_READ_BATCH: usize = 8_192;

pub const PREVIEW_ROWS: usize = 5;
pub const PREVIEW_COLS: usize = 5;

pub const MAX_FIELD_CHARS: usize = 255;

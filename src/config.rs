pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_BATCH_SIZE: u32 = 50_000;

// Default table name for source and destination. The value "main" means
// "not set", so a wrong explicit -n fails loudly instead of falling back.
pub const DEFAULT_TABLE_NAME: &str = "main";

// SQLite bound-parameter ceiling; multi-row INSERTs are split under it.
pub const SQLITE_MAX_PARAMS: usize = 32_766;

// A negative cache_size (in KiB) keeps pages in RAM, not on disk.
pub const SQLITE_CACHE_KIB: i64 = 65_536;

// FTS5 bulk-load tuning: segment merging is deferred while rows are loaded
// and done once at the end; the defaults are restored afterwards.
pub const FTS5_BULK_CRISISMERGE: i64 = 64;
pub const FTS5_DEFAULT_AUTOMERGE: i64 = 4;
pub const FTS5_DEFAULT_CRISISMERGE: i64 = 16;

// Memory-mapped I/O window for reading large SQLite databases (2 GiB).
pub const SQLITE_MMAP_BYTES: i64 = 2 << 30;

// Sequential read buffer for CSV/text input.
pub const READ_BUFFER_BYTES: usize = 1 << 20;

// Buffered writer capacity for Parquet output.
pub const WRITE_BUFFER_BYTES: usize = 1 << 20;

// Batch size for the streaming Parquet record reader.
pub const PARQUET_READ_BATCH: usize = 8_192;

// Rows per batch handed from the reader thread to the writer, and how many
// batches may wait in between (bounds memory to DEPTH x BATCH rows).
pub const PIPELINE_BATCH_ROWS: usize = 4_096;
pub const PIPELINE_DEPTH: usize = 4;
// A batch is also cut at this many bytes of cell text, so very wide rows
// cannot multiply the memory held in flight.
pub const PIPELINE_BATCH_BYTES: usize = 8 << 20;

// Bounded buffer between the SQLite reader task and the writer iterator.
pub const SQLITE_STREAM_CHANNEL: usize = 8_192;

// Default rows sampled to infer column types (--infer-rows).
pub const INFER_SAMPLE_ROWS: u32 = 8_192;

// A tuple is a runaway parse when it has more fields than the larger of
// (expected fields times FACTOR) and MIN.
pub const SQL_TUPLE_FIELD_FACTOR: usize = 16;
pub const SQL_TUPLE_FIELD_MIN: usize = 4_096;

// Malformed-row and type-mismatch warnings printed before the rest are suppressed.
pub const SKIP_WARN_LIMIT: u64 = 10;

pub const PREVIEW_ROWS: usize = 5;
pub const PREVIEW_COLS: usize = 5;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, bail};
use arrow::array::{ArrayRef, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{BrotliLevel, Compression as ParquetCompression, GzipLevel, ZstdLevel};
use parquet::file::properties::WriterProperties;

use crate::app::readers;
use crate::app::transform;
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::ui;
use crate::args::{AppArgs, Compression};
use crate::config;

pub async fn parquet_processing(args: &AppArgs) -> Result<()> {
    let out_path = if args.output_path.ends_with(".parquet") {
        args.output_path.clone()
    } else {
        format!("{}.parquet", args.output_path)
    };

    ui::section("parquet");

    match args.input_file_type.as_str() {
        "csv" | "txt" => csv_to_parquet(args, &out_path).await?,
        "json" | "jsonl" | "ndjson" => json_to_parquet(args, &out_path).await?,
        "sqlite" => sqlite_to_parquet(args, &out_path).await?,
        other => bail!("Parquet output supports csv/txt/json/sqlite input, got: {other}"),
    }

    verify_output(&out_path)?;
    Ok(())
}

async fn csv_to_parquet(args: &AppArgs, out_path: &str) -> Result<()> {
    let encoding = match &args.encoding {
        Some(enc) => enc.clone(),
        None => transform::encoding::detect_encoding(&args.input_path)?,
    };
    ui::field("charset", &encoding);

    let has_headers = !args.no_header;
    let headers =
        readers::csv_parse::csv_headers(&args.input_path, args.delimiter, &encoding, has_headers)
            .await?;
    let column_names = unique_column_names(&headers);

    let rows = readers::csv_parse::csv_row_reader(
        args.input_path.clone(),
        args.delimiter,
        &encoding,
        has_headers,
    )
    .await?;

    write_parquet(args, out_path, column_names, rows, None).await
}

async fn json_to_parquet(args: &AppArgs, out_path: &str) -> Result<()> {
    let headers = readers::json_parse::json_schema(&args.input_path)?;
    let column_names = unique_column_names(&headers);

    let rows = readers::json_parse::json_row_reader(args.input_path.clone(), column_names.clone())?;

    write_parquet(args, out_path, column_names, rows, None).await
}

async fn sqlite_to_parquet(args: &AppArgs, out_path: &str) -> Result<()> {
    let (table, headers, data) =
        readers::sqlite_parse::read_table(&args.input_path, &args.table_name).await?;
    ui::field("table", &table);

    let column_names = unique_column_names(&headers);
    let total = data.len() as u64;
    let rows = data.into_iter().map(Ok);

    write_parquet(args, out_path, column_names, rows, Some(total)).await
}

async fn write_parquet(
    args: &AppArgs,
    out_path: &str,
    column_names: Vec<String>,
    rows: impl Iterator<Item = Result<Vec<String>>>,
    total_rows: Option<u64>,
) -> Result<()> {
    let row_group_size = args.batch_size.max(1) as usize;

    let col_count = column_names.len();
    if col_count == 0 {
        bail!("input has no columns");
    }
    ui::field("columns", &col_count.to_string());

    let fields: Vec<Field> = column_names
        .iter()
        .map(|name| Field::new(name, DataType::Utf8, true))
        .collect();
    let schema = Arc::new(Schema::new(fields));

    let props = WriterProperties::builder()
        .set_compression(map_compression(args.compression, args.level)?)
        .set_max_row_group_row_count(Some(row_group_size))
        .set_write_batch_size(row_group_size)
        .build();

    let file = BufWriter::with_capacity(config::WRITE_BUFFER_BYTES, File::create(out_path)?);
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
    ui::step(&format!(
        "writing {}{} → {out_path}",
        format!("{:?}", args.compression).to_lowercase(),
        args.level.map(|l| format!(":{l}")).unwrap_or_default()
    ));

    let started = Instant::now();
    let mut columns: Vec<Vec<Option<String>>> = (0..col_count)
        .map(|_| Vec::with_capacity(row_group_size))
        .collect();
    let mut in_batch = 0usize;
    let mut total = 0u64;
    let mut preview_shown = false;
    let mut preview_buf: Vec<Vec<String>> = Vec::new();

    for row_result in rows {
        match row_result {
            Ok(record) => {
                if !preview_shown {
                    preview_buf.push(record.clone());
                    if preview_buf.len() >= config::PREVIEW_ROWS {
                        process_and_pause(std::mem::take(&mut preview_buf)).await?;
                        preview_shown = true;
                    }
                }

                let mut fields = record.into_iter();
                for slot in columns.iter_mut() {
                    slot.push(fields.next());
                }
                in_batch += 1;
                total += 1;

                if in_batch >= row_group_size {
                    write_batch(&mut writer, &schema, &mut columns)?;
                    in_batch = 0;
                    ui::progress("write", total, total_rows);
                }
            }
            Err(e) => ui::error(&format!("failed to read row: {e}")),
        }
    }

    if !preview_shown && !preview_buf.is_empty() {
        process_and_pause(std::mem::take(&mut preview_buf)).await?;
    }

    if in_batch > 0 {
        write_batch(&mut writer, &schema, &mut columns)?;
    }

    writer.close()?;
    ui::progress_done("wrote", total, &format!("{:.2?}", started.elapsed()));
    Ok(())
}

fn write_batch<W: Write + Send>(
    writer: &mut ArrowWriter<W>,
    schema: &Arc<Schema>,
    columns: &mut [Vec<Option<String>>],
) -> Result<()> {
    let arrays: Vec<ArrayRef> = columns
        .iter_mut()
        .map(|col| {
            let arr: StringArray = std::mem::take(col).into_iter().collect();
            Arc::new(arr) as ArrayRef
        })
        .collect();

    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    writer.write(&batch)?;
    Ok(())
}

fn verify_output(out_path: &str) -> Result<()> {
    ui::section("verify");

    let (names, total_rows) = readers::parquet_parse::schema_columns(out_path)?;
    ui::field("rows", &total_rows.to_string());
    ui::field("columns", &names.len().to_string());

    if total_rows == 0 {
        ui::warn("output has no data rows");
        return Ok(());
    }

    let last = (total_rows - 1) as i64;
    let mid = pseudo_random_mid(total_rows);
    let mut wanted: Vec<i64> = vec![0, 1.min(last), mid, last];
    wanted.sort_unstable();
    wanted.dedup();
    wanted.retain(|&i| i >= 0 && i < total_rows as i64);

    let sample = readers::parquet_parse::read_rows(out_path, &wanted)?;
    if sample.is_empty() {
        bail!("integrity check failed: could not read back any row");
    }

    let mut header = names.clone();
    header.truncate(config::PREVIEW_COLS);
    let mut table = vec![header];
    table.extend(sample.iter().cloned());

    ui::preview_table(&table, table.len(), config::PREVIEW_COLS);
    ui::ok("integrity check passed (head/mid/tail readable)");
    Ok(())
}

fn pseudo_random_mid(total_rows: usize) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    (now % total_rows) as i64
}

fn map_compression(c: Compression, level: Option<i32>) -> Result<ParquetCompression> {
    Ok(match c {
        Compression::Zstd => {
            let lvl = level.unwrap_or(3);
            ParquetCompression::ZSTD(ZstdLevel::try_new(lvl)?)
        }
        Compression::Snappy => ParquetCompression::SNAPPY,
        Compression::Gzip => {
            let lvl = level.unwrap_or(6) as u32;
            ParquetCompression::GZIP(GzipLevel::try_new(lvl)?)
        }
        Compression::Brotli => {
            let lvl = level.unwrap_or(5) as u32;
            ParquetCompression::BROTLI(BrotliLevel::try_new(lvl)?)
        }
        Compression::Lz4 => ParquetCompression::LZ4,
        Compression::None => ParquetCompression::UNCOMPRESSED,
    })
}

fn unique_column_names(headers: &[String]) -> Vec<String> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut names = Vec::with_capacity(headers.len());

    for (i, header) in headers.iter().enumerate() {
        let base = if header.trim().is_empty() {
            format!("c{i}")
        } else {
            header.trim().to_string()
        };

        let counter = seen.entry(base.clone()).or_insert(0);
        let name = if *counter == 0 {
            base.clone()
        } else {
            format!("{base}_{counter}")
        };
        *counter += 1;
        names.push(name);
    }

    names
}

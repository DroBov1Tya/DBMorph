use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

use anyhow::{bail, Result};
use csv::WriterBuilder;

use crate::app::readers;
use crate::app::transform;
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::ui;
use crate::args::AppArgs;
use crate::config;

pub async fn csv_processing(args: &AppArgs) -> Result<()> {
    let out_path = if args.output_path.ends_with(".csv") {
        args.output_path.clone()
    } else {
        format!("{}.csv", args.output_path)
    };

    ui::section("csv");

    match args.input_file_type.as_str() {
        "parquet" => parquet_to_csv(args, &out_path).await?,
        "sqlite" => sqlite_to_csv(args, &out_path).await?,
        "csv" | "txt" => csv_to_csv(args, &out_path).await?,
        other => bail!("CSV output supports parquet/sqlite/csv/txt input, got: {other}"),
    }

    Ok(())
}

async fn parquet_to_csv(args: &AppArgs, out_path: &str) -> Result<()> {
    let (headers, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("rows", &total_rows.to_string());

    let rows = readers::parquet_parse::parquet_row_reader(&args.input_path)?;
    write_all(out_path, headers, args.delimiter, rows, Some(total_rows as u64)).await
}

async fn sqlite_to_csv(args: &AppArgs, out_path: &str) -> Result<()> {
    let (table, headers, data) =
        readers::sqlite_parse::read_table(&args.input_path, &args.table_name).await?;
    ui::field("table", &table);

    let total = data.len() as u64;
    let rows = data.into_iter().map(Ok);
    write_all(out_path, headers, args.delimiter, rows, Some(total)).await
}

async fn csv_to_csv(args: &AppArgs, out_path: &str) -> Result<()> {
    let encoding = match &args.encoding {
        Some(enc) => enc.clone(),
        None => transform::encoding::detect_encoding(&args.input_path)?,
    };
    ui::field("charset", &encoding);

    let has_headers = !args.no_header;
    let headers =
        readers::csv_parse::csv_headers(&args.input_path, args.delimiter, &encoding, has_headers)
            .await?;
    let rows = readers::csv_parse::csv_row_reader(
        args.input_path.clone(),
        args.delimiter,
        &encoding,
        has_headers,
    )
    .await?;

    write_all(out_path, headers, args.delimiter, rows, None).await
}

async fn write_all(
    out_path: &str,
    headers: Vec<String>,
    delimiter: u8,
    rows: impl Iterator<Item = Result<Vec<String>>>,
    total_rows: Option<u64>,
) -> Result<()> {
    let file = BufWriter::with_capacity(config::WRITE_BUFFER_BYTES, File::create(out_path)?);
    let mut writer = WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(file);

    ui::field("columns", &headers.len().to_string());
    writer.write_record(&headers)?;
    ui::step(&format!("writing → {out_path}"));

    let cadence = config::DEFAULT_BATCH_SIZE.max(1) as u64;
    let started = Instant::now();
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

                writer.write_record(&record)?;
                total += 1;

                if total.is_multiple_of(cadence) {
                    ui::progress("write", total, total_rows);
                }
            }
            Err(e) => ui::error(&format!("failed to read row: {e}")),
        }
    }

    if !preview_shown && !preview_buf.is_empty() {
        process_and_pause(std::mem::take(&mut preview_buf)).await?;
    }

    writer.flush()?;
    ui::progress_done("wrote", total, &format!("{:.2?}", started.elapsed()));
    Ok(())
}

use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

use anyhow::{Result, bail};
use csv::WriterBuilder;

use crate::app::readers;
use crate::app::transform;
use crate::app::transform::rows::{RowGuard, guarded, pipelined};
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::paths::ensure_distinct_paths;
use crate::app::utils::progress::Progress;
use crate::app::utils::ui;
use crate::args::AppArgs;
use crate::config;

pub async fn csv_processing(args: &AppArgs) -> Result<u64> {
    let out_path = if args.output_path.ends_with(".csv") {
        args.output_path.clone()
    } else {
        format!("{}.csv", args.output_path)
    };

    ensure_distinct_paths(
        std::path::Path::new(&args.input_path),
        std::path::Path::new(&out_path),
    )?;

    ui::section("csv");

    let skipped = match args.input_file_type.as_str() {
        "parquet" => parquet_to_csv(args, &out_path).await?,
        "sqlite" => sqlite_to_csv(args, &out_path).await?,
        "json" | "jsonl" | "ndjson" => json_to_csv(args, &out_path).await?,
        "xlsx" | "xls" | "ods" => xlsx_to_csv(args, &out_path).await?,
        "sql" | "dump" => sql_to_csv(args, &out_path).await?,
        "csv" | "txt" => csv_to_csv(args, &out_path).await?,
        other => {
            bail!("CSV output supports parquet/sqlite/json/xlsx/sql/csv/txt input, got: {other}")
        }
    };

    Ok(skipped)
}

async fn parquet_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let (auto, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("rows", &total_rows.to_string());
    let headers = transform::columns::resolve_output_names(&auto, args.columns.as_deref())?;

    let raw = readers::parquet_parse::parquet_row_reader(std::path::Path::new(&args.input_path))?;
    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(raw, guard.clone()))?,
        &Progress::rows(total_rows as u64),
    )
    .await?;
    Ok(guard.finish())
}

async fn sqlite_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let (table, auto) = readers::sqlite_parse::schema(&args.input_path, &args.table_name).await?;
    ui::field("table", &table);

    let total = if args.count {
        Some(readers::sqlite_parse::count_rows(&args.input_path, &table).await?)
    } else {
        None
    };

    let headers = transform::columns::resolve_output_names(&auto, args.columns.as_deref())?;
    // Columns are selected by their real names, not the cleaned output names.
    let raw = readers::sqlite_parse::stream_rows(args.input_path.clone(), table, auto);
    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(raw, guard.clone()))?,
        &total.map_or_else(Progress::unknown, Progress::rows),
    )
    .await?;
    Ok(guard.finish())
}

async fn json_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let auto = readers::json_parse::json_schema(&args.input_path)?;
    let headers = transform::columns::resolve_output_names(&auto, args.columns.as_deref())?;
    ui::field("columns", &headers.len().to_string());

    let progress = Progress::file(&args.input_path);
    // Rows are looked up by the original JSON keys, not the cleaned names.
    let raw = readers::json_parse::json_row_reader(args.input_path.clone(), auto, &progress)?;
    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(raw, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish())
}

async fn xlsx_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let path = std::path::Path::new(&args.input_path);
    let has_headers = !args.no_header;
    let auto = readers::xlsx_parse::xlsx_headers(path, has_headers)?;
    let headers = transform::columns::resolve_output_names(&auto, args.columns.as_deref())?;
    ui::field("columns", &headers.len().to_string());

    let raw = readers::xlsx_parse::xlsx_row_reader(path, has_headers)?;
    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(raw, guard.clone()))?,
        &Progress::unknown(),
    )
    .await?;
    Ok(guard.finish())
}

async fn sql_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let wanted = if args.table_name == "main" {
        None
    } else {
        Some(args.table_name.as_str())
    };

    let progress = Progress::file(&args.input_path);
    let (source_table, auto, raw) =
        readers::sql_parse::sql_open(&args.input_path, wanted, &progress)?;
    ui::field("source table", &source_table);
    let headers = transform::columns::resolve_output_names(&auto, args.columns.as_deref())?;
    ui::field("columns", &headers.len().to_string());

    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(raw, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish())
}

async fn csv_to_csv(args: &AppArgs, out_path: &str) -> Result<u64> {
    let progress = Progress::file(&args.input_path);
    let source = readers::csv_parse::open_csv(args, &progress).await?;
    let headers =
        transform::columns::resolve_output_names(&source.headers, args.columns.as_deref())?;

    let guard = RowGuard::for_args(headers.len(), args);
    write_all(
        out_path,
        headers,
        args.delimiter,
        pipelined(guarded(source.rows, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish() + source.stats.finish())
}

async fn write_all(
    out_path: &str,
    headers: Vec<String>,
    delimiter: u8,
    rows: impl Iterator<Item = Result<Vec<String>>>,
    progress: &Progress,
) -> Result<()> {
    write_all_with_preview(
        out_path,
        headers,
        delimiter,
        rows,
        progress,
        process_and_pause,
    )
    .await
}

async fn write_all_with_preview<P, Fut>(
    out_path: &str,
    headers: Vec<String>,
    delimiter: u8,
    mut rows: impl Iterator<Item = Result<Vec<String>>>,
    progress: &Progress,
    preview_fn: P,
) -> Result<()>
where
    P: FnOnce(Vec<Vec<String>>) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut sample = Vec::with_capacity(config::PREVIEW_ROWS);
    for row in rows.by_ref() {
        sample.push(row?);
        if sample.len() >= config::PREVIEW_ROWS {
            break;
        }
    }
    if !sample.is_empty() {
        preview_fn(sample.clone()).await?;
    }

    // Opening with File::create truncates an existing file; wait until after
    // preview confirmation so cancellation has no destination side effect.
    let file = BufWriter::with_capacity(config::WRITE_BUFFER_BYTES, File::create(out_path)?);
    let mut writer = WriterBuilder::new().delimiter(delimiter).from_writer(file);

    ui::field("columns", &headers.len().to_string());
    writer.write_record(&headers)?;
    ui::step(&format!("writing -> {out_path}"));

    let cadence = config::DEFAULT_BATCH_SIZE.max(1) as u64;
    let started = Instant::now();
    let progress = &progress.restarted();
    let mut total = 0u64;
    for row_result in sample.into_iter().map(Ok).chain(rows) {
        let record = row_result?;
        writer.write_record(&record)?;
        total += 1;

        if total.is_multiple_of(cadence) {
            ui::progress("write", total, progress);
        }
    }

    writer.flush()?;
    ui::progress_done("wrote", total, &format!("{:.2?}", started.elapsed()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dbm-csv-{name}-{}-{unique}.csv",
            std::process::id()
        ))
    }

    async fn declined_preview(path: &std::path::Path) -> Result<()> {
        write_all_with_preview(
            path.to_str().unwrap(),
            vec!["column".to_owned()],
            b',',
            vec![Ok(vec!["value".to_owned()])].into_iter(),
            &Progress::unknown(),
            |_| async { anyhow::bail!("preview declined") },
        )
        .await
    }

    #[tokio::test]
    async fn declining_preview_leaves_destination_absent() {
        let path = test_path("absent");
        assert!(!path.exists());
        assert!(declined_preview(&path).await.is_err());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn declining_preview_preserves_existing_destination() {
        let path = test_path("existing");
        let original = b"keep this output intact\n";
        std::fs::write(&path, original).unwrap();
        assert!(declined_preview(&path).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn same_input_and_output_path_is_rejected_without_truncation() {
        let input = test_path("same");
        let original = b"name\nAda\n";
        std::fs::write(&input, original).unwrap();
        let output_base = input.with_extension("");
        let args = AppArgs::try_parse_from([
            "DBMorph",
            "-i",
            input.to_str().unwrap(),
            "-o",
            output_base.to_str().unwrap(),
            "-f",
            "csv",
            "-t",
            "csv",
        ])
        .unwrap();

        assert!(csv_processing(&args).await.is_err());
        assert_eq!(std::fs::read(&input).unwrap(), original);
        std::fs::remove_file(input).unwrap();
    }
}

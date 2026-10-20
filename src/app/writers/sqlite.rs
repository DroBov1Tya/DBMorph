use std::future::Future;
use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

use crate::app::readers;
use crate::app::transform;
use crate::app::transform::infer::{self, ColType};
use crate::app::transform::rows::{RowGuard, guarded, pipelined};
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::paths::ensure_distinct_paths;
use crate::app::utils::progress::Progress;
use crate::app::utils::ui;
use crate::args::{AppArgs, SqliteMode};
use crate::config;

mod requests;

use requests::TableSpec;

pub async fn sqlite_processing(args: &AppArgs) -> Result<u64> {
    let db_path = format!("sqlite://{}.db", args.output_path);
    ensure_distinct_paths(
        std::path::Path::new(&args.input_path),
        std::path::Path::new(&format!("{}.db", args.output_path)),
    )?;

    ui::section("sqlite");
    if args.infer_types && args.sqlite_mode == SqliteMode::Fts {
        ui::warn("--infer-types has no effect on FTS5 tables; use --sqlite-mode typed");
    }
    let skipped = match args.input_file_type.as_str() {
        "csv" | "txt" => ingest_csv(&db_path, args).await?,
        "parquet" => ingest_parquet(&db_path, args).await?,
        "json" | "jsonl" | "ndjson" => ingest_json(&db_path, args).await?,
        "xlsx" | "xls" | "ods" => ingest_xlsx(&db_path, args).await?,
        "sql" | "dump" => ingest_sql(&db_path, args).await?,
        other => {
            ui::warn(&format!("Unsupported input file type: {other}"));
            0
        }
    };

    Ok(skipped)
}

async fn ingest_csv(db_path: &str, args: &AppArgs) -> Result<u64> {
    let progress = Progress::file(&args.input_path);
    let source = readers::csv_parse::open_csv(args, &progress).await?;
    let names = transform::columns::resolve_output_names(&source.headers, args.columns.as_deref())?;

    let guard = RowGuard::for_args(names.len(), args);
    let mismatched = load_table(
        db_path,
        args,
        &args.table_name,
        &names,
        pipelined(guarded(source.rows, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish() + mismatched + source.stats.finish())
}

async fn ingest_json(db_path: &str, args: &AppArgs) -> Result<u64> {
    let headers = readers::json_parse::json_schema(&args.input_path)?;
    ui::field("columns", &headers.len().to_string());

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let progress = Progress::file(&args.input_path);
    // Rows are looked up by the original JSON keys, not the cleaned names.
    let raw = readers::json_parse::json_row_reader(args.input_path.clone(), headers, &progress)?;

    let guard = RowGuard::for_args(names.len(), args);
    let mismatched = load_table(
        db_path,
        args,
        &args.table_name,
        &names,
        pipelined(guarded(raw, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_xlsx(db_path: &str, args: &AppArgs) -> Result<u64> {
    let path = std::path::Path::new(&args.input_path);
    let has_headers = !args.no_header;
    let headers = readers::xlsx_parse::xlsx_headers(path, has_headers)?;
    ui::field("columns", &headers.len().to_string());

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let raw = readers::xlsx_parse::xlsx_row_reader(path, has_headers)?;

    let guard = RowGuard::for_args(names.len(), args);
    let mismatched = load_table(
        db_path,
        args,
        &args.table_name,
        &names,
        pipelined(guarded(raw, guard.clone()))?,
        &Progress::unknown(),
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_parquet(db_path: &str, args: &AppArgs) -> Result<u64> {
    let (names, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("columns", &names.len().to_string());

    let names = transform::columns::resolve_output_names(&names, args.columns.as_deref())?;

    let raw = readers::parquet_parse::parquet_row_reader(std::path::Path::new(&args.input_path))?;

    let guard = RowGuard::for_args(names.len(), args);
    let mismatched = load_table(
        db_path,
        args,
        &args.table_name,
        &names,
        pipelined(guarded(raw, guard.clone()))?,
        &Progress::rows(total_rows as u64),
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_sql(db_path: &str, args: &AppArgs) -> Result<u64> {
    let wanted = if args.table_name == "main" {
        None
    } else {
        Some(args.table_name.as_str())
    };

    let progress = Progress::file(&args.input_path);
    let (source_table, headers, raw) =
        readers::sql_parse::sql_open(&args.input_path, wanted, &progress)?;
    ui::field("source table", &source_table);
    ui::field("columns", &headers.len().to_string());

    let dest_table = if args.table_name == "main" {
        source_table.clone()
    } else {
        args.table_name.clone()
    };

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let guard = RowGuard::for_args(names.len(), args);
    let mismatched = load_table(
        db_path,
        args,
        &dest_table,
        &names,
        pipelined(guarded(raw, guard.clone()))?,
        &progress,
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

// Shared tail of every ingest: reads a sample, shows the preview, creates the
// table in the chosen layout and streams all rows in. The table is created
// only after the preview is confirmed, because a typed layout needs the
// sample to pick column types. Returns cells that did not fit their type.
async fn load_table(
    db_path: &str,
    args: &AppArgs,
    table_name: &str,
    columns: &[String],
    rows: impl Iterator<Item = Result<Vec<String>>>,
    progress: &Progress,
) -> Result<u64> {
    load_table_with_preview(
        db_path,
        args,
        table_name,
        columns,
        rows,
        progress,
        process_and_pause,
    )
    .await
}

async fn load_table_with_preview<P, Fut>(
    db_path: &str,
    args: &AppArgs,
    table_name: &str,
    columns: &[String],
    mut rows: impl Iterator<Item = Result<Vec<String>>>,
    progress: &Progress,
    preview_fn: P,
) -> Result<u64>
where
    P: FnOnce(Vec<Vec<String>>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let columns = &transform::columns::unique_column_names_nocase(columns);

    let sample_target = match args.sqlite_mode {
        SqliteMode::Fts => config::PREVIEW_ROWS,
        SqliteMode::Typed => args.infer_rows as usize,
    };

    let mut sample: Vec<Vec<String>> = Vec::with_capacity(sample_target);
    for row in rows.by_ref() {
        sample.push(row?);
        if sample.len() >= sample_target {
            break;
        }
    }

    if !sample.is_empty() {
        let preview: Vec<Vec<String>> = sample.iter().take(config::PREVIEW_ROWS).cloned().collect();
        preview_fn(preview).await?;
    }

    // Do not create or touch the destination until the preview is accepted.
    ui::step(&format!("connecting -> {db_path}"));
    let connect_opts = SqliteConnectOptions::from_str(db_path)?.create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&connect_opts).await?;
    ui::ok("connected");
    if args.drop_existing {
        requests::delete_exists_table(&mut conn, table_name).await?;
    }

    let types = match args.sqlite_mode {
        SqliteMode::Fts => vec![ColType::Text; columns.len()],
        SqliteMode::Typed => {
            let t = infer::infer_types(&sample, columns.len());
            ui::field("inferred", &infer::describe_types(&t));
            t
        }
    };

    let spec = TableSpec {
        name: table_name,
        columns,
        types: &types,
    };
    let fts5_bulk = match args.sqlite_mode {
        SqliteMode::Fts => requests::create_fts5_table(&mut conn, &spec).await?,
        SqliteMode::Typed => {
            requests::create_typed_table(&mut conn, &spec).await?;
            false
        }
    };
    if fts5_bulk {
        requests::fts5_bulk_load_begin(&mut conn, table_name).await?;
    }

    let loaded = requests::init_insert_process(
        &mut conn,
        &spec,
        args.batch_size,
        args.strict,
        sample.into_iter().map(Ok).chain(rows),
        progress,
    )
    .await;

    if fts5_bulk {
        finish_fts5_bulk_load(&mut conn, table_name, loaded.is_ok()).await?;
    }
    loaded
}

// Undoes the bulk-load tuning. After a failed load the rows inserted so far
// are committed and the defaults restored, so the table is never left with
// merging switched off; the original error is what the caller reports.
async fn finish_fts5_bulk_load(
    conn: &mut SqliteConnection,
    table_name: &str,
    loaded: bool,
) -> Result<()> {
    if loaded {
        requests::fts5_restore_defaults(conn, table_name).await?;
        return requests::fts5_optimize(conn, table_name).await;
    }

    // The transaction may already be closed; a failing COMMIT changes nothing.
    let _ = sqlx::query("COMMIT;").execute(&mut *conn).await;
    if let Err(e) = requests::fts5_restore_defaults(conn, table_name).await {
        ui::warn(&format!(
            "could not restore FTS5 settings of '{table_name}' ({e}); re-run with -x"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn declining_preview_does_not_create_sqlite_destination() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let output = std::env::temp_dir().join(format!(
            "dbm-preview-cancel-{}-{unique}",
            std::process::id()
        ));
        let db_path = format!("sqlite://{}.db", output.display());
        let target = std::path::PathBuf::from(format!("{}.db", output.display()));
        assert!(!target.exists());

        let args = AppArgs::try_parse_from(["DBMorph", "-i", "input.csv", "-o", "unused"]).unwrap();
        let rows = vec![Ok(vec!["row".to_owned()])].into_iter();
        let result = load_table_with_preview(
            &db_path,
            &args,
            "data",
            &["column".to_owned()],
            rows,
            &Progress::unknown(),
            |_| async { anyhow::bail!("preview declined") },
        )
        .await;

        assert!(result.is_err());
        assert!(
            !target.exists(),
            "declining preview must leave no target file"
        );
    }
}

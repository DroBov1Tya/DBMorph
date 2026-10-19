use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

use crate::app::readers;
use crate::app::transform;
use crate::app::transform::infer::{self, ColType};
use crate::app::transform::rows::{RowGuard, guarded};
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::ui;
use crate::args::{AppArgs, SqliteMode};
use crate::config;

mod requests;

use requests::TableSpec;

pub async fn sqlite_processing(args: &AppArgs) -> Result<u64> {
    let db_path = format!("sqlite://{}.db", args.output_path);

    ui::section("sqlite");
    if args.infer_types && args.sqlite_mode == SqliteMode::Fts {
        ui::warn("--infer-types has no effect on FTS5 tables; use --sqlite-mode typed");
    }
    ui::step(&format!("connecting -> {db_path}"));

    let connect_opts = SqliteConnectOptions::from_str(&db_path)?.create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&connect_opts).await?;
    ui::ok("connected");

    if args.drop_existing {
        requests::delete_exists_table(&mut conn, &args.table_name).await?;
    }

    let skipped = match args.input_file_type.as_str() {
        "csv" | "txt" => ingest_csv(&mut conn, args).await?,
        "parquet" => ingest_parquet(&mut conn, args).await?,
        "json" | "jsonl" | "ndjson" => ingest_json(&mut conn, args).await?,
        "xlsx" | "xls" | "ods" => ingest_xlsx(&mut conn, args).await?,
        "sql" | "dump" => ingest_sql(&mut conn, args).await?,
        other => {
            ui::warn(&format!("Unsupported input file type: {other}"));
            0
        }
    };

    Ok(skipped)
}

async fn ingest_csv(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let encoding = match &args.encoding {
        Some(enc) => enc.clone(),
        None => transform::encoding::detect_encoding(&args.input_path)?,
    };
    ui::field("charset", &encoding);

    let has_headers = !args.no_header;
    let quoting = !args.no_quote;
    let repair = !args.no_repair;
    let headers = readers::csv_parse::csv_headers(
        &args.input_path,
        args.delimiter,
        &encoding,
        has_headers,
        quoting,
        repair,
    )
    .await?;

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let (raw, repairs) = readers::csv_parse::csv_row_reader(
        args.input_path.clone(),
        args.delimiter,
        &encoding,
        has_headers,
        quoting,
        repair,
    )
    .await?;

    let guard = RowGuard::new(names.len(), args.max_field, args.strict, &args.trim);
    let mismatched = load_table(
        conn,
        args,
        &args.table_name,
        &names,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish() + mismatched + readers::csv_parse::report_repairs(&repairs))
}

async fn ingest_json(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let headers = readers::json_parse::json_schema(&args.input_path)?;
    ui::field("columns", &headers.len().to_string());

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    // Rows are looked up by the original JSON keys, not the cleaned names.
    let raw = readers::json_parse::json_row_reader(args.input_path.clone(), headers)?;

    let guard = RowGuard::new(names.len(), args.max_field, args.strict, &args.trim);
    let mismatched = load_table(
        conn,
        args,
        &args.table_name,
        &names,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_xlsx(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let path = std::path::Path::new(&args.input_path);
    let has_headers = !args.no_header;
    let headers = readers::xlsx_parse::xlsx_headers(path, has_headers)?;
    ui::field("columns", &headers.len().to_string());

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let raw = readers::xlsx_parse::xlsx_row_reader(path, has_headers)?;

    let guard = RowGuard::new(names.len(), args.max_field, args.strict, &args.trim);
    let mismatched = load_table(
        conn,
        args,
        &args.table_name,
        &names,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_parquet(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let (names, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("columns", &names.len().to_string());

    let names = transform::columns::resolve_output_names(&names, args.columns.as_deref())?;

    let raw = readers::parquet_parse::parquet_row_reader(&args.input_path)?;

    let guard = RowGuard::new(names.len(), args.max_field, args.strict, &args.trim);
    let mismatched = load_table(
        conn,
        args,
        &args.table_name,
        &names,
        guarded(raw, guard.clone()),
        Some(total_rows as u64),
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

async fn ingest_sql(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let wanted = if args.table_name == "main" {
        None
    } else {
        Some(args.table_name.as_str())
    };

    let (source_table, headers, raw) = readers::sql_parse::sql_open(&args.input_path, wanted)?;
    ui::field("source table", &source_table);
    ui::field("columns", &headers.len().to_string());

    let dest_table = if args.table_name == "main" {
        source_table.clone()
    } else {
        args.table_name.clone()
    };

    let names = transform::columns::resolve_output_names(&headers, args.columns.as_deref())?;

    let guard = RowGuard::new(names.len(), args.max_field, args.strict, &args.trim);
    let mismatched = load_table(
        conn,
        args,
        &dest_table,
        &names,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish() + mismatched)
}

// Shared tail of every ingest: reads a sample, shows the preview, creates the
// table in the chosen layout and streams all rows in. The table is created
// only after the preview is confirmed, because a typed layout needs the
// sample to pick column types. Returns cells that did not fit their type.
async fn load_table(
    conn: &mut SqliteConnection,
    args: &AppArgs,
    table_name: &str,
    columns: &[String],
    mut rows: impl Iterator<Item = Result<Vec<String>>>,
    total_rows: Option<u64>,
) -> Result<u64> {
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
        process_and_pause(preview).await?;
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
    match args.sqlite_mode {
        SqliteMode::Fts => requests::create_fts5_table(conn, &spec).await?,
        SqliteMode::Typed => requests::create_typed_table(conn, &spec).await?,
    }

    requests::init_insert_process(
        conn,
        &spec,
        args.batch_size,
        args.strict,
        sample.into_iter().map(Ok).chain(rows),
        total_rows,
    )
    .await
}

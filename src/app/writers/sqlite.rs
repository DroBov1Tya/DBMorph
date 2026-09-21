use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

use crate::app::readers;
use crate::app::transform;
use crate::app::transform::rows::{RowGuard, guarded};
use crate::app::utils::ui;
use crate::args::AppArgs;

mod requests;

pub async fn sqlite_processing(args: &AppArgs) -> Result<u64> {
    let db_path = format!("sqlite://{}.db", args.output_path);

    ui::section("sqlite");
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
    let headers = readers::csv_parse::csv_headers(
        &args.input_path,
        args.delimiter,
        &encoding,
        has_headers,
        quoting,
    )
    .await?;

    let columns = requests::create_fts5_table(conn, &args.table_name, &headers).await?;

    let raw = readers::csv_parse::csv_row_reader(
        args.input_path.clone(),
        args.delimiter,
        &encoding,
        has_headers,
        quoting,
    )
    .await?;

    let guard = RowGuard::new(columns.len(), args.max_field, args.strict);
    requests::init_insert_process(
        conn,
        &args.table_name,
        columns,
        args.batch_size,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish())
}

async fn ingest_json(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let headers = readers::json_parse::json_schema(&args.input_path)?;
    ui::field("columns", &headers.len().to_string());

    let columns = requests::create_fts5_table(conn, &args.table_name, &headers).await?;

    let raw = readers::json_parse::json_row_reader(args.input_path.clone(), columns.clone())?;

    let guard = RowGuard::new(columns.len(), args.max_field, args.strict);
    requests::init_insert_process(
        conn,
        &args.table_name,
        columns,
        args.batch_size,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish())
}

async fn ingest_parquet(conn: &mut SqliteConnection, args: &AppArgs) -> Result<u64> {
    let (names, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("columns", &names.len().to_string());

    let columns = requests::create_fts5_table(conn, &args.table_name, &names).await?;

    let raw = readers::parquet_parse::parquet_row_reader(&args.input_path)?;

    let guard = RowGuard::new(columns.len(), args.max_field, args.strict);
    requests::init_insert_process(
        conn,
        &args.table_name,
        columns,
        args.batch_size,
        guarded(raw, guard.clone()),
        Some(total_rows as u64),
    )
    .await?;
    Ok(guard.finish())
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

    let columns = requests::create_fts5_table(conn, &dest_table, &headers).await?;

    let guard = RowGuard::new(columns.len(), args.max_field, args.strict);
    requests::init_insert_process(
        conn,
        &dest_table,
        columns,
        args.batch_size,
        guarded(raw, guard.clone()),
        None,
    )
    .await?;
    Ok(guard.finish())
}

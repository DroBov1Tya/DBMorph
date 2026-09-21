use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

use crate::app::readers;
use crate::app::transform;
use crate::app::utils::ui;
use crate::args::AppArgs;

mod requests;

pub async fn sqlite_processing(args: &AppArgs) -> Result<()> {
    let db_path = format!("sqlite://{}.db", args.output_path);

    ui::section("sqlite");
    ui::step(&format!("connecting → {db_path}"));

    let connect_opts = SqliteConnectOptions::from_str(&db_path)?.create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&connect_opts).await?;
    ui::ok("connected");

    if args.drop_existing {
        requests::delete_exists_table(&mut conn, &args.table_name).await?;
    }

    match args.input_file_type.as_str() {
        "csv" | "txt" => ingest_csv(&mut conn, args).await?,
        "parquet" => ingest_parquet(&mut conn, args).await?,
        "sql" => {
            readers::sql_parse::parse_dump_simple(args.input_path.clone()).await?;
        }
        other => ui::warn(&format!("Unsupported input file type: {other}")),
    }

    Ok(())
}

async fn ingest_csv(conn: &mut SqliteConnection, args: &AppArgs) -> Result<()> {
    let encoding = match &args.encoding {
        Some(enc) => enc.clone(),
        None => transform::encoding::detect_encoding(&args.input_path)?,
    };
    ui::field("charset", &encoding);

    let has_headers = !args.no_header;
    let headers =
        readers::csv_parse::csv_headers(&args.input_path, args.delimiter, &encoding, has_headers)
            .await?;

    let columns = requests::create_fts5_table(conn, &args.table_name, &headers).await?;

    let rows = readers::csv_parse::csv_row_reader(
        args.input_path.clone(),
        args.delimiter,
        &encoding,
        has_headers,
    )
    .await?;

    requests::init_insert_process(conn, &args.table_name, columns, args.batch_size, rows, None)
        .await
}

async fn ingest_parquet(conn: &mut SqliteConnection, args: &AppArgs) -> Result<()> {
    let (names, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("columns", &names.len().to_string());

    let columns = requests::create_fts5_table(conn, &args.table_name, &names).await?;

    let rows = readers::parquet_parse::parquet_row_reader(&args.input_path)?;

    requests::init_insert_process(
        conn,
        &args.table_name,
        columns,
        args.batch_size,
        rows,
        Some(total_rows as u64),
    )
    .await
}

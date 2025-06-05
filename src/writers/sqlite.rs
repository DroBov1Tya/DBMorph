use colored::*;
use sqlx::{sqlite::SqliteConnectOptions, Connection, SqliteConnection};
use std::process;
use std::{error::Error, str::FromStr};
use tracing::{error, info, warn};

use crate::{readers, transform};
mod csv_insert;
mod json_insert;
mod sql_insert;
mod sqlite_init;

pub async fn sqlite_processing(
    input_file: String,
    output_file: String,
    table_name: String,
    column_count: i32,
    encoding: Option<String>,
    delimiter: Option<u8>,
    remove_rows: Option<usize>,
    custom_rows: Option<String>,
    drop_existing: bool,
    batch_size: Option<u32>,
    input_file_type: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Checks batch size limit and exits if exceeded
    // Determines input file extension from argument or filename
    // Connects to SQLite database, creating it if missing
    // Optionally drops existing table if specified
    // Detects or uses provided file encoding
    // Based on file extension, parses and processes data:
    // Warns if file type is unsupported
    if let Some(size) = batch_size {
        if size > 1000 {
            warn!(
                "{} SQLite batch size must not exceed 1000",
                "⚠️ [WARN]".yellow().bold()
            );
            process::exit(1);
        }
    }

    let extension = if let Some(file_type) = input_file_type {
        file_type
            .split('.')
            .last()
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else {
        let ext = input_file
            .split('.')
            .last()
            .map(|s| s.to_string())
            .unwrap_or_default();
        println!("{}    Found extension: {}", "✅ [AUTO]".green().bold(), ext);
        ext
    };

    let db_path = format!("sqlite://{}.db", output_file);

    println!(
        "{}   Connecting to database at: {}",
        "🔄 [DB]".blue().bold(),
        db_path
    );

    let connect_opts = SqliteConnectOptions::from_str(&db_path)?.create_if_missing(true);
    let mut sqlite_conn = SqliteConnection::connect_with(&connect_opts).await?;

    println!(
        "{}   Connected to database successfully",
        "✅ [DB]".green().bold()
    );

    if drop_existing == true {
        let _delete_exists_table =
            sqlite_init::delete_exists_table(&mut sqlite_conn, &table_name).await;
    }

    let encoding = match encoding {
        Some(enc) => enc,
        _ => transform::encoding::detect_encoding(&input_file).unwrap(),
    };

    println!(
        "{}    Detected file charset: {}",
        "✅ [INPUT]".green().bold(),
        &encoding
    );

    match extension.as_str() {
        "csv" | "txt" => {
            let lines_count = readers::csv_parse::count_lines(&input_file, remove_rows).await?;
            let column_count = readers::csv_parse::check_max_collumns(
                &input_file,
                &encoding,
                delimiter.unwrap(),
                remove_rows,
            )
            .await
            .unwrap_or(column_count);

            let columns =
                sqlite_init::create_fts5_table(&mut sqlite_conn, &table_name, column_count).await;

            let all_rows = readers::csv_parse::csv_row_reader(
                input_file.clone(),
                &encoding,
                delimiter.unwrap(),
                remove_rows,
            )
            .await
            .unwrap();

            let _start_process = csv_insert::init_insert_process(
                &mut sqlite_conn,
                &table_name,
                columns.unwrap(),
                batch_size,
                all_rows,
                lines_count,
                custom_rows,
            )
            .await;
        }
        "json" => {
            let lines_count = readers::json_parse::count_objects_with_keys(&input_file).await?;
            let column_count = readers::json_parse::count_keys_in_first_json(&input_file)
                .await
                .unwrap_or(column_count);

            let columns =
                sqlite_init::create_fts5_table(&mut sqlite_conn, &table_name, column_count).await;

            let json_stream = readers::json_parse::read_json_lines_flat(input_file).await;
            let _start_process = json_insert::processing_json(
                &mut sqlite_conn,
                &table_name,
                columns.unwrap(),
                json_stream,
                batch_size,
                lines_count,
            )
            .await?;
        }
        "xlsx" => {}
        "sql" => {
            let target_table_name = readers::sql_parse::extract_table_names(&input_file).await?;
            let total_rows =
                readers::sql_parse::count_rows_in_table(&input_file, &target_table_name).await?;
            let columns_count =
                readers::sql_parse::count_columns_in_first_row(&input_file, &target_table_name)
                    .await?;
            let columns =
                sqlite_init::create_fts5_table(&mut sqlite_conn, &table_name, columns_count).await;

            let _start_process = sql_insert::init_sql(
                &mut sqlite_conn,
                input_file,
                table_name,
                target_table_name,
                columns.unwrap(),
                encoding,
                batch_size,
                total_rows,
            )
            .await?;
        }
        _ => {
            warn!(
                "{} Unable to detect input file type",
                "⚠️ [WARN]".yellow().bold()
            );
        }
    }
    Ok(())
}

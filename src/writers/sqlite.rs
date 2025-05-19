use colored::*;
use sqlx::{sqlite::SqliteConnectOptions, Connection, SqliteConnection};
use std::process;
use std::{error::Error, str::FromStr};
use tracing::{error, info, warn};

use crate::{readers, transform};
mod requests;

pub async fn sqlite_processing(
    input_file: String,
    output_file: String,
    table_name: String,
    encoding: Option<String>,
    drop_existing: bool,
    batch_size: Option<u32>,
    delimiter: Option<u8>,
    inpun_file_type: Option<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(size) = batch_size {
        if size > 1000 {
            warn!(
                "{} SQLite batch size must not exceed 1000",
                "⚠️ [WARN]".yellow().bold()
            );
            process::exit(1);
        }
    }

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
            requests::delete_exists_table(&mut sqlite_conn, &table_name).await;
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

    match inpun_file_type.unwrap().as_str() {
        "csv" | "txt" => {
            let lines_count = readers::csv_parse::count_lines(&input_file).await?;
            let column_count: Result<i32, Box<dyn Error + 'static>> =
                readers::csv_parse::check_max_collumns(&input_file, delimiter.unwrap(), &encoding)
                    .await;

            let columns =
                requests::create_fts5_table(&mut sqlite_conn, &table_name, column_count.unwrap())
                    .await;

            let all_rows = readers::csv_parse::csv_row_reader(
                input_file.clone(),
                delimiter.unwrap(),
                &encoding,
            )
            .await
            .unwrap();
            let _start_process = requests::init_insert_process(
                &mut sqlite_conn,
                &table_name,
                columns.unwrap(),
                batch_size,
                all_rows,
                lines_count,
            )
            .await;
        }
        "json" => {}
        "xlsx" => {}
        "sql" => {
            let check = readers::sql_parse::parse_dump_simple(input_file).await;
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

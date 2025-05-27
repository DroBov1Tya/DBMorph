use colored::*;
use std::error::Error;
use std::path::Path;
use std::process;
use tracing::{error, info, warn};

use crate::{readers, transform};
mod csv_insert;
mod mongo_init;
mod sqlite_insert;

pub async fn mongodb_processing(
    input_file: String,
    output_file: String,
    table_name: String,
    column_count: i32,
    encoding: Option<String>,
    delimiter: Option<u8>,
    remove_rows: Option<usize>,
    drop_existing: bool,
    headers_row: bool,
    database_url: Option<String>,
    database_user: Option<String>,
    database_pass: Option<String>,
    batch_size: Option<u32>,
    input_file_type: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Connects to MongoDB with given credentials or exits if missing
    // Determines collection name from input file stem and extension
    // Optionally drops existing collection if specified
    // Detects or uses provided file encoding
    // Processes input file differently based on extension (csv, json, xlsx, sql)
    // Warns if file type is unsupported
    let client = if let (Some(url), Some(user), Some(pass)) =
        (&database_url, &database_user, &database_pass)
    {
        mongo_init::mongo_connect(url, user, pass).await?
    } else {
        warn!(
            "{}",
            "Не все параметры подключения заданы. Завершаем работу."
                .yellow()
                .bold()
        );
        process::exit(1);
    };

    let path = Path::new(&input_file);
    let collection_name = path.file_stem().unwrap().to_str().unwrap();

    let extension = if let Some(file_type) = input_file_type {
        file_type
            .split('.')
            .last()
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else {
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_string()
    };

    let db = client.database(&output_file);
    let collection = db.collection::<mongodb::bson::Document>(&collection_name);

    if drop_existing == true {
        let _delete_exists_table = &collection.drop().await;
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

            let all_rows = readers::csv_parse::csv_row_reader(
                input_file.clone(),
                &encoding,
                delimiter.unwrap(),
                remove_rows,
            )
            .await
            .unwrap();

            let _start_process = csv_insert::init_insert_to_mongo(
                &collection,
                batch_size,
                all_rows,
                lines_count,
                headers_row,
            )
            .await?;
        }
        "json" => {
            let lines_count = readers::json_parse::count_objects_with_keys(&input_file).await?;
            let column_count = readers::json_parse::count_keys_in_first_json(&input_file)
                .await
                .unwrap_or(column_count);
        }
        "xlsx" => {}
        "sql" => {
            let target_table_name = readers::sql_parse::extract_table_names(&input_file).await?;
            let total_rows =
                readers::sql_parse::count_rows_in_table(&input_file, &target_table_name).await?;
            let columns_count =
                readers::sql_parse::count_columns_in_first_row(&input_file, &target_table_name)
                    .await?;
        }
        "db" | "sqlite" => {
            let mut sqlite_conn = readers::sqlite_parse::init_connect(&output_file).await?;

            let total_rows =
                readers::sqlite_parse::count_rows_in_table(&input_file, &table_name).await?;

            let reader =
                readers::sqlite_parse::stream_sqlite_rows(&mut sqlite_conn, &table_name).await?;

            let _ = sqlite_insert::insert_stream_to_mongo(
                &collection,
                batch_size,
                reader,
                total_rows,
                headers_row,
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

use clap::builder::Str;
use csv::StringRecord;
use rusqlite::{params_from_iter, Connection, Result, ToSql};
use std::error::Error;
use tracing::info;

use crate::utils::question::process_and_pause;

pub fn create_fts5_table(
    database_name: String,
    table_name: String,
    column_count: i32,
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut conn = Connection::open(format!("./{}.db", database_name))?;
    let tx = conn.transaction()?;

    let column_names: Vec<String> = (0..column_count)
    .map(|i| format!("c{}", i))
    .collect();

    let column_definitions_str = column_names.join(", ");
    let create_table_sql = format!("CREATE VIRTUAL TABLE IF NOT EXISTS \"{}\" USING fts5({}, tokenize ='unicode61')",
        table_name,
        column_definitions_str
    );

    let _ = tx.execute(&create_table_sql, [])?;
    let _ = tx.commit();
    info!("[+] Successfuly created SQLite databse with {} columns in table {}", column_count, table_name);
    Ok(column_names)
}

pub fn delete_exists_table(
    database_name: String,
    table_name: String,
) -> Result<(), Box<dyn Error>> {
    let mut conn = Connection::open(format!("./{}.db", database_name))?;
    let tx = conn.transaction()?;

    let delete_sql = "DROP TABLE IF EXISTS $1";

    let _ = tx.execute(delete_sql, [table_name.clone()]);
    let _ = tx.commit();

    info!("[+] Successfult deleted existing table {}", table_name);

    Ok(())
}

pub fn init_insert_process(
    database_name: String,
    table_name: String,
    columns: Vec<String>,
    batch_size: Option<u32>,
    all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
) -> Result<(), Box<dyn Error>> {
    let mut conn = Connection::open(format!("./{}.db", database_name))?;
    let preview_count = 5;
    let batch_size = batch_size.unwrap().try_into().unwrap();
    let mut preview_shown = false;
    let mut total_rows = 0;

    let mut chunk: Vec<Vec<String>> = Vec::with_capacity(batch_size);

    for row_result in all_rows {
        match row_result {
            Ok(record) => {
                let fields: Vec<String> = record.iter().map(|s| s.to_string()).collect();
                chunk.push(fields);
                total_rows += 1;

                if !preview_shown && chunk.len() == preview_count {
                let preview = chunk[..preview_count].to_vec();
                let _ = process_and_pause(preview);
                preview_shown = true;
            }

                if chunk.len() >= batch_size {
                    insert_batch_to_sqlite(&mut conn, &table_name, &columns, &chunk)?;
                    chunk.clear();
                    println!("[+] Строк обработано: {}", &total_rows)
                }
            }
            Err(e) => {
                eprintln!("Ошибка при чтении строки CSV: {}", e);
            }
        }
    }

    if !chunk.is_empty() {
        insert_batch_to_sqlite(&mut conn, &table_name, &columns, &chunk)?;
        println!("[+] Строк обработано: {}", &total_rows)
    }

    Ok(())
}

fn insert_batch_to_sqlite(
    conn: &mut Connection,
    table_name: &String,
    columns: &Vec<String>,
    chunk: &Vec<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    if chunk.is_empty() {
        return Ok(());
    }

    let tx = conn.transaction()?;

    let placeholders = vec!["?"; columns.len()].join(", ");
    let column_str = columns.join(", ");
    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({})",
        table_name, column_str, placeholders
    );

    {
    let mut stmt = tx.prepare_cached(&insert_sql)?;

    for row in chunk {
        let mut formatted_row: Vec<String> = if row.len() < columns.len() {
            let mut padded = row.clone();
            padded.resize(columns.len(), "".to_string());
            padded
        } else {
            row[..columns.len()].to_vec()
        };

        formatted_row = formatted_row
            .into_iter()
            .map(|f| truncate_field(&f))
            .collect();

        let values: Vec<&dyn ToSql> = formatted_row.iter().map(|s| s as &dyn ToSql).collect();
        stmt.execute(params_from_iter(values))?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn truncate_field(input: &str) -> String {
    const MAX_LEN: usize = 255;

    if input.chars().count() > MAX_LEN {
        input.chars().take(MAX_LEN).collect()
    } else {
        input.to_string()
    }
}
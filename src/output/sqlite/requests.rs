use csv::StringRecord;
use rusqlite::{params_from_iter, Connection, Result, ToSql, TransactionBehavior};
use std::error::Error;
use tracing::info;

use crate::utils::question::process_and_pause;

pub async fn create_fts5_table(
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

pub async fn delete_exists_table(
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

pub async fn init_insert_process(
    database_name: String,
    table_name: String,
    columns: Vec<String>,
    batch_size: Option<u32>,
    all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
) -> Result<(), Box<dyn Error>> {
    let mut conn = Connection::open(format!("./{}.db", database_name))?;
    let _mode: String = conn.query_row(
    "PRAGMA journal_mode = WAL;", 
    [], 
    |row| row.get(0)
    )?;
    conn.execute("PRAGMA synchronous = OFF;", [])?;

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
                    insert_batch_to_sqlite(&mut conn, &table_name, &columns, &chunk).await?;
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
        insert_batch_to_sqlite(&mut conn, &table_name, &columns, &chunk).await?;
        println!("[+] Строк обработано: {}", &total_rows)
    }

    Ok(())
}

async fn insert_batch_to_sqlite(
    conn: &mut Connection,
    table_name: &String,
    columns: &Vec<String>,
    chunk: &Vec<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    if chunk.is_empty() {
        return Ok(());
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    
    let target_column_count = columns.len();
    if target_column_count == 0 {
        return Err("Cannot insert into a table with zero columns defined in 'columns_fts'.".into());
    }

    let placeholders = vec!["?"; target_column_count].join(", ");
    let column_names_str = columns.join(", ");
    let sanitized_table_name = sanitize_identifier(table_name);

    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({})",
        sanitized_table_name, column_names_str, placeholders
    );

    {
        let mut stmt = tx.prepare_cached(&insert_sql)?;
        let mut values_for_binding: Vec<String> = Vec::with_capacity(target_column_count);

        for row_in_chunk in chunk {
            values_for_binding.clear();

            for i in 0..target_column_count {
                if let Some(field_str_ref) = row_in_chunk.get(i) {
                    values_for_binding.push(truncate_field_string_owned(field_str_ref));
                } else {
                    values_for_binding.push(String::new());
                }
            }
            
            stmt.execute(params_from_iter(
                values_for_binding.iter().map(|s_val| s_val as &dyn ToSql),
            ))?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn truncate_field_string_owned(input: &str) -> String {
    const MAX_CHARS: usize = 255;

    let mut char_iter = input.char_indices();
    let mut char_count = 0;
    let mut limit_byte_idx = input.len();

    while let Some((idx, _)) = char_iter.next() {
        char_count += 1;
        if char_count > MAX_CHARS {
            limit_byte_idx = idx;
            break;
        }
    }
    
    if limit_byte_idx == input.len() {
        input.to_string()
    } else {
        input[..limit_byte_idx].to_string()
    }
}

fn sanitize_identifier(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect()
}
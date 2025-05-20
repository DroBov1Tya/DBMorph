use colored::*;
use sqlx::{sqlite::SqliteQueryResult, SqliteConnection};
use std::error::Error;
use tracing::info;

pub async fn create_fts5_table(
    conn: &mut SqliteConnection,
    table_name: &String,
    column_count: i32,
) -> Result<Vec<String>, Box<dyn Error>> {
    let column_names: Vec<String> = (0..column_count).map(|i| format!("c{}", i)).collect();
    let column_definitions_str = column_names.join(", ");

    let create_table_sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS \"{}\" USING fts5({}, tokenize ='unicode61')",
        table_name, column_definitions_str
    );

    sqlx::query(&create_table_sql).execute(conn).await?;

    println!(
        "{}    Created SQLite FTS5 table '{}' with {} columns",
        "✅ [DB]".green().bold(),
        table_name,
        column_count
    );

    Ok(column_names)
}

pub async fn delete_exists_table(
    conn: &mut SqliteConnection,
    table_name: &String,
) -> Result<(), Box<dyn Error>> {
    let delete_sql = format!("DROP TABLE IF EXISTS \"{}\"", table_name);

    sqlx::query(&delete_sql)
        .bind(&table_name)
        .execute(&mut *conn)
        .await?;

    println!(
        "{}   Existing table '{}' dropped",
        "✅ [DB]".green().bold(),
        table_name
    );

    Ok(())
}

pub async fn insert_chunk_to_sqlite(
    conn: &mut SqliteConnection,
    table_name: &String,
    columns: &Vec<String>,
    chunk: &Vec<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    if chunk.is_empty() {
        println!("[insert] Empty chunk");
        return Ok(());
    }

    let target_column_count = columns.len();
    if target_column_count == 0 {
        return Err("Cannot insert into a table with zero columns.".into());
    }

    let sanitized_table_name = sanitize_identifier(table_name).await;
    let column_names_str = columns.join(", ");
    let mut placeholders = Vec::new();
    for _ in chunk {
        let row_placeholders = std::iter::repeat("?")
            .take(target_column_count)
            .collect::<Vec<_>>()
            .join(", ");
        placeholders.push(format!("({})", row_placeholders));
    }

    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES {}",
        sanitized_table_name,
        column_names_str,
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&insert_sql);

    for row_in_chunk in chunk {
        for i in 0..target_column_count {
            let val = row_in_chunk.get(i).map(|s| s.as_str()).unwrap_or("");
            let truncated_val = truncate_field_string_owned(val).await;
            query = query.bind(truncated_val);
        }
    }

    let _result: SqliteQueryResult = query.execute(conn).await?;

    Ok(())
}

async fn truncate_field_string_owned(input: &str) -> String {
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

async fn sanitize_identifier(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

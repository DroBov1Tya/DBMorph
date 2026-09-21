use std::time::Instant;

use anyhow::Result;
use sqlx::SqliteConnection;

use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::ui;
use crate::config;

pub async fn create_fts5_table(
    conn: &mut SqliteConnection,
    table_name: &str,
    headers: &[String],
) -> Result<Vec<String>> {
    let column_names = unique_column_names(headers);
    let column_definitions_str = column_names
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");

    let create_table_sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS \"{table_name}\" USING fts5({column_definitions_str}, tokenize ='unicode61')"
    );

    sqlx::query(&create_table_sql).execute(&mut *conn).await?;
    ui::ok(&format!(
        "created FTS5 table '{table_name}' ({} cols)",
        column_names.len()
    ));

    Ok(column_names)
}

pub async fn delete_exists_table(conn: &mut SqliteConnection, table_name: &str) -> Result<()> {
    let delete_sql = format!("DROP TABLE IF EXISTS \"{table_name}\"");
    sqlx::query(&delete_sql).execute(&mut *conn).await?;
    ui::ok(&format!("dropped existing table '{table_name}'"));
    Ok(())
}

async fn apply_fast_pragmas(conn: &mut SqliteConnection) -> Result<()> {
    let mode: (String,) = sqlx::query_as("PRAGMA journal_mode = OFF;")
        .fetch_one(&mut *conn)
        .await?;

    for pragma in [
        "PRAGMA synchronous = OFF;",
        "PRAGMA locking_mode = EXCLUSIVE;",
        "PRAGMA temp_store = MEMORY;",
        &format!("PRAGMA cache_size = -{};", config::SQLITE_CACHE_KIB),
        "PRAGMA mmap_size = 268435456;",
    ] {
        sqlx::query(pragma).execute(&mut *conn).await?;
    }

    ui::info(&format!(
        "journal={} sync=off temp=mem cache={}MiB",
        mode.0,
        config::SQLITE_CACHE_KIB / 1024
    ));
    Ok(())
}

pub async fn init_insert_process(
    conn: &mut SqliteConnection,
    table_name: &str,
    columns: Vec<String>,
    batch_size: u32,
    rows: impl Iterator<Item = Result<Vec<String>>>,
    total_rows: Option<u64>,
) -> Result<()> {
    apply_fast_pragmas(conn).await?;

    let col_count = columns.len();
    if col_count == 0 {
        anyhow::bail!("cannot insert into a table with zero columns");
    }

    let param_cap = (config::SQLITE_MAX_PARAMS / col_count).max(1);
    let effective_batch = (batch_size.max(1) as usize).min(param_cap);

    let table = sanitize_identifier(table_name);
    let column_names_str = columns
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");
    let full_sql = build_insert_sql(&table, &column_names_str, col_count, effective_batch);

    let started = Instant::now();
    let mut preview_shown = false;
    let mut preview_buf: Vec<Vec<String>> = Vec::new();
    let mut total = 0u64;
    let mut chunk: Vec<Vec<String>> = Vec::with_capacity(effective_batch);

    sqlx::query("BEGIN;").execute(&mut *conn).await?;

    for row_result in rows {
        let fields = row_result?;

        if !preview_shown {
            preview_buf.push(fields.clone());
            if preview_buf.len() >= config::PREVIEW_ROWS {
                process_and_pause(std::mem::take(&mut preview_buf)).await?;
                preview_shown = true;
            }
        }

        chunk.push(fields);
        total += 1;

        if chunk.len() >= effective_batch {
            insert_batch(conn, &full_sql, col_count, &chunk).await?;
            chunk.clear();
            ui::progress("insert", total, total_rows);
        }
    }

    if !preview_shown && !preview_buf.is_empty() {
        process_and_pause(std::mem::take(&mut preview_buf)).await?;
    }

    if !chunk.is_empty() {
        let tail_sql = build_insert_sql(&table, &column_names_str, col_count, chunk.len());
        insert_batch(conn, &tail_sql, col_count, &chunk).await?;
    }

    sqlx::query("COMMIT;").execute(&mut *conn).await?;

    ui::progress_done("inserted", total, &format!("{:.2?}", started.elapsed()));
    Ok(())
}

async fn insert_batch(
    conn: &mut SqliteConnection,
    sql: &str,
    col_count: usize,
    chunk: &[Vec<String>],
) -> Result<()> {
    if chunk.is_empty() {
        return Ok(());
    }

    let mut query = sqlx::query(sql);
    for row in chunk {
        for i in 0..col_count {
            let val = row.get(i).map(|s| s.as_str()).unwrap_or("");
            query = query.bind(val);
        }
    }

    query.execute(&mut *conn).await?;
    Ok(())
}

fn build_insert_sql(table: &str, columns: &str, col_count: usize, rows: usize) -> String {
    let row_placeholders = format!("({})", vec!["?"; col_count].join(", "));
    let mut values = String::with_capacity(rows * (col_count * 3 + 2));
    for i in 0..rows {
        if i > 0 {
            values.push(',');
        }
        values.push_str(&row_placeholders);
    }
    format!("INSERT INTO \"{table}\" ({columns}) VALUES {values}")
}

fn sanitize_identifier(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unique_column_names(headers: &[String]) -> Vec<String> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut names = Vec::with_capacity(headers.len());

    for (i, header) in headers.iter().enumerate() {
        let base = if header.trim().is_empty() {
            format!("c{i}")
        } else {
            header.trim().to_string()
        };

        let counter = seen.entry(base.clone()).or_insert(0);
        let name = if *counter == 0 {
            base.clone()
        } else {
            format!("{base}_{counter}")
        };
        *counter += 1;
        names.push(name);
    }

    names
}

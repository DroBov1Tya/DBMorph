use std::str::FromStr;

use anyhow::{Result, bail};
use futures::TryStreamExt;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection};

async fn connect(db_file: &str) -> Result<SqliteConnection> {
    let path = if db_file.ends_with(".db") {
        db_file.to_string()
    } else {
        format!("{db_file}.db")
    };
    let url = format!("sqlite://{path}");
    let opts = SqliteConnectOptions::from_str(&url)?.create_if_missing(false);
    Ok(SqliteConnection::connect_with(&opts).await?)
}

async fn resolve_table(conn: &mut SqliteConnection, requested: &str) -> Result<String> {
    let exists: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
    )
    .bind(requested)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some((name,)) = exists {
        return Ok(name);
    }

    let first: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type IN ('table','view') \
         AND name NOT LIKE 'sqlite_%' ORDER BY rowid LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;

    match first {
        Some((name,)) => Ok(name),
        None => bail!("no tables found in SQLite database"),
    }
}

async fn column_names(conn: &mut SqliteConnection, table: &str) -> Result<Vec<String>> {
    let rows = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
        .fetch_all(&mut *conn)
        .await?;

    let mut names = Vec::with_capacity(rows.len());
    for row in rows {
        names.push(row.try_get::<String, _>("name")?);
    }

    if names.is_empty() {
        bail!("table '{table}' has no columns");
    }
    Ok(names)
}

async fn row_count(conn: &mut SqliteConnection, table: &str) -> Result<u64> {
    let count: (i64,) = sqlx::query_as(&format!("SELECT count(*) FROM \"{table}\""))
        .fetch_one(&mut *conn)
        .await?;
    Ok(count.0.max(0) as u64)
}

/// Reads a SQLite table fully into memory as string rows, preserving the real
/// schema column names as the returned header.
pub async fn read_table(
    db_file: &str,
    requested_table: &str,
) -> Result<(String, Vec<String>, Vec<Vec<String>>)> {
    let mut conn = connect(db_file).await?;
    let table = resolve_table(&mut conn, requested_table).await?;
    let columns = column_names(&mut conn, &table).await?;
    let total = row_count(&mut conn, &table).await?;

    let select_list = columns
        .iter()
        .map(|c| format!("CAST(\"{c}\" AS TEXT)"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select_list} FROM \"{table}\"");

    let mut out: Vec<Vec<String>> = Vec::with_capacity(total as usize);
    let mut stream = sqlx::query(&sql).fetch(&mut conn);

    while let Some(row) = stream.try_next().await? {
        let mut record = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let value: Option<String> = row.try_get(i)?;
            record.push(value.unwrap_or_default());
        }
        out.push(record);
    }

    Ok((table, columns, out))
}

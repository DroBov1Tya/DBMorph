use std::str::FromStr;
use std::sync::mpsc::{Receiver, sync_channel};

use anyhow::{Result, bail};
use futures::TryStreamExt;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection};

use crate::config;

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

/// Resolves the real table name, schema columns and row count without reading data.
pub async fn schema(db_file: &str, requested_table: &str) -> Result<(String, Vec<String>, u64)> {
    let mut conn = connect(db_file).await?;
    let table = resolve_table(&mut conn, requested_table).await?;
    let columns = column_names(&mut conn, &table).await?;
    let total = row_count(&mut conn, &table).await?;
    Ok((table, columns, total))
}

/// Streams a SQLite table as string rows without materializing it: a dedicated
/// thread drives the async query and feeds a bounded channel read synchronously.
pub fn stream_rows(
    db_file: String,
    table: String,
    columns: Vec<String>,
) -> impl Iterator<Item = Result<Vec<String>>> {
    let (tx, rx) = sync_channel::<Result<Vec<String>>>(config::SQLITE_STREAM_CHANNEL);

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Err(e.into()));
                return;
            }
        };

        rt.block_on(async move {
            if let Err(e) = pump(&db_file, &table, &columns, &tx).await {
                let _ = tx.send(Err(e));
            }
        });
    });

    RowStream { rx }
}

async fn pump(
    db_file: &str,
    table: &str,
    columns: &[String],
    tx: &std::sync::mpsc::SyncSender<Result<Vec<String>>>,
) -> Result<()> {
    let mut conn = connect(db_file).await?;

    let select_list = columns
        .iter()
        .map(|c| format!("CAST(\"{c}\" AS TEXT)"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select_list} FROM \"{table}\"");

    let mut stream = sqlx::query(&sql).fetch(&mut conn);
    while let Some(row) = stream.try_next().await? {
        let mut record = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let value: Option<String> = row.try_get(i)?;
            record.push(value.unwrap_or_default());
        }
        if tx.send(Ok(record)).is_err() {
            break;
        }
    }
    Ok(())
}

struct RowStream {
    rx: Receiver<Result<Vec<String>>>,
}

impl Iterator for RowStream {
    type Item = Result<Vec<String>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rx.recv().ok()
    }
}

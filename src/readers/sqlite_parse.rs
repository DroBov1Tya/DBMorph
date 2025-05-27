use colored::*;
use futures::{stream::BoxStream, Stream, StreamExt};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteRow},
    Connection, Row, SqliteConnection,
};
use std::{collections::BTreeMap, error::Error, str::FromStr};

pub async fn init_connect(input_file: &String) -> Result<SqliteConnection, Box<dyn Error>> {
    let db_path = format!("sqlite://{}.db", input_file);

    println!(
        "{}   Connecting to sqlite database at: {}",
        "🔄 [DB]".blue().bold(),
        db_path
    );

    let connect_opts = SqliteConnectOptions::from_str(&db_path)?.create_if_missing(true);
    let conn = SqliteConnection::connect_with(&connect_opts).await?;

    println!(
        "{}   Connected to sqlite database successfully",
        "✅ [DB]".green().bold()
    );

    Ok(conn)
}

pub async fn count_rows_in_table(
    input_file: &String,
    table: &String,
) -> Result<i64, Box<dyn Error>> {
    let db_path = format!("sqlite://{}", input_file);
    let connect_opts = SqliteConnectOptions::from_str(&db_path)?.create_if_missing(true);
    let mut sqlite_conn = SqliteConnection::connect_with(&connect_opts).await?;
    let query = format!("SELECT COUNT(*) FROM {}", table);

    let row: (i64,) = sqlx::query_as(&query).fetch_one(&mut sqlite_conn).await?;

    Ok(row.0)
}

pub async fn stream_sqlite_rows<'c>(
    conn: &'c mut SqliteConnection,
    table: &String,
) -> Result<BoxStream<'c, Result<Vec<String>, Box<dyn Error + Send + Sync>>>, Box<dyn Error>> {
    let query: &'static str = Box::leak(format!("SELECT * FROM {}", table).into_boxed_str());

    let stream = sqlx::query(&query)
        .fetch(conn)
        .map(|row_res| match row_res {
            Ok(row) => {
                let mut row_vec = Vec::new();
                for i in 0..row.len() {
                    match row.try_get::<String, _>(i) {
                        Ok(val) => row_vec.push(val),
                        Err(_) => row_vec.push(String::new()),
                    }
                }
                Ok(row_vec)
            }
            Err(e) => Err(Box::new(e) as Box<dyn Error + Send + Sync>),
        })
        .boxed();

    Ok(stream)
}

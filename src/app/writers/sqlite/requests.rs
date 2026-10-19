use std::time::Instant;

use anyhow::{Result, bail};
use sqlx::SqliteConnection;
use sqlx::sqlite::SqliteArguments;

use crate::app::transform::columns::quote_identifier;
use crate::app::transform::infer::{self, Cell, ColType, Mismatches};
use crate::app::utils::ui;
use crate::config;

type SqliteQuery<'q> = sqlx::query::Query<'q, sqlx::Sqlite, SqliteArguments<'q>>;

// Destination table: its name, column names and the type each column's cells
// are bound with (all Text for FTS5).
pub struct TableSpec<'a> {
    pub name: &'a str,
    pub columns: &'a [String],
    pub types: &'a [ColType],
}

pub async fn create_fts5_table(conn: &mut SqliteConnection, spec: &TableSpec<'_>) -> Result<()> {
    sqlx::query(&fts5_table_sql(spec))
        .execute(&mut *conn)
        .await?;
    ui::ok(&format!(
        "created FTS5 table '{}' ({} cols)",
        spec.name,
        spec.columns.len()
    ));
    Ok(())
}

pub async fn create_typed_table(conn: &mut SqliteConnection, spec: &TableSpec<'_>) -> Result<()> {
    sqlx::query(&typed_table_sql(spec))
        .execute(&mut *conn)
        .await?;
    ui::ok(&format!(
        "created STRICT table '{}' ({} cols)",
        spec.name,
        spec.columns.len()
    ));
    Ok(())
}

pub async fn delete_exists_table(conn: &mut SqliteConnection, table_name: &str) -> Result<()> {
    let delete_sql = format!("DROP TABLE IF EXISTS {}", quote_identifier(table_name));
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

// Streams rows into an existing table with multi-row INSERTs inside one
// transaction. Each cell is bound with its column type; a cell that does not
// fit is stored as NULL and counted (an error in strict mode). Returns the
// number of such cells.
pub async fn init_insert_process(
    conn: &mut SqliteConnection,
    spec: &TableSpec<'_>,
    batch_size: u32,
    strict: bool,
    rows: impl Iterator<Item = Result<Vec<String>>>,
    total_rows: Option<u64>,
) -> Result<u64> {
    apply_fast_pragmas(conn).await?;

    let col_count = spec.columns.len();
    if col_count == 0 || spec.types.len() != col_count {
        bail!("cannot insert into a table with zero columns or mismatched types");
    }

    let param_cap = (config::SQLITE_MAX_PARAMS / col_count).max(1);
    let effective_batch = (batch_size.max(1) as usize).min(param_cap);
    let full_sql = build_insert_sql(spec, effective_batch);

    let started = Instant::now();
    let mut inserter = Inserter {
        spec,
        strict,
        first_row: 0,
        mismatches: Mismatches::new(col_count),
    };
    let mut total = 0u64;
    let mut chunk: Vec<Vec<String>> = Vec::with_capacity(effective_batch);

    sqlx::query("BEGIN;").execute(&mut *conn).await?;

    for row_result in rows {
        chunk.push(row_result?);
        total += 1;

        if chunk.len() >= effective_batch {
            inserter.insert(conn, &full_sql, &chunk).await?;
            chunk.clear();
            ui::progress("insert", total, total_rows);
        }
    }

    if !chunk.is_empty() {
        let tail_sql = build_insert_sql(spec, chunk.len());
        inserter.insert(conn, &tail_sql, &chunk).await?;
    }

    sqlx::query("COMMIT;").execute(&mut *conn).await?;

    ui::progress_done("inserted", total, &format!("{:.2?}", started.elapsed()));
    Ok(inserter.mismatches.finish(spec.columns, spec.types))
}

// Per-run insert state: the target table and the cells that did not fit.
struct Inserter<'a> {
    spec: &'a TableSpec<'a>,
    strict: bool,
    first_row: u64,
    mismatches: Mismatches,
}

impl Inserter<'_> {
    async fn insert(
        &mut self,
        conn: &mut SqliteConnection,
        sql: &str,
        chunk: &[Vec<String>],
    ) -> Result<()> {
        let types = self.spec.types;
        let mut query = sqlx::query(sql);
        for (offset, row) in chunk.iter().enumerate() {
            for (col, ty) in types.iter().enumerate() {
                let raw = row.get(col).map(String::as_str).unwrap_or("");
                let cell = match infer::coerce(*ty, raw) {
                    Some(cell) => cell,
                    None => self.mismatch(self.first_row + offset as u64, col)?,
                };
                query = bind_cell(query, cell);
            }
        }

        query.execute(&mut *conn).await?;
        self.first_row += chunk.len() as u64;
        Ok(())
    }

    // The value itself is left out of the message: input may hold personal data.
    fn mismatch(&mut self, row: u64, col: usize) -> Result<Cell<'static>> {
        if self.strict {
            bail!(
                "inserted row {}: column '{}' does not fit {} (inferred from the sample); \
                 raise --infer-rows or drop --strict to store such values as NULL",
                row + 1,
                self.spec.columns[col],
                self.spec.types[col].sqlite()
            );
        }
        self.mismatches.add(col, 1);
        Ok(Cell::Null)
    }
}

fn bind_cell<'q>(query: SqliteQuery<'q>, cell: Cell<'q>) -> SqliteQuery<'q> {
    match cell {
        Cell::Null => query.bind(None::<i64>),
        Cell::Int(n) => query.bind(n),
        Cell::Float(f) => query.bind(f),
        Cell::Bool(b) => query.bind(i64::from(b)),
        Cell::Text(s) => query.bind(s),
    }
}

fn column_list(columns: &[String]) -> String {
    columns
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fts5_table_sql(spec: &TableSpec<'_>) -> String {
    format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING fts5({}, tokenize ='unicode61')",
        quote_identifier(spec.name),
        column_list(spec.columns)
    )
}

fn typed_table_sql(spec: &TableSpec<'_>) -> String {
    let definitions = spec
        .columns
        .iter()
        .zip(spec.types)
        .map(|(c, ty)| format!("{} {}", quote_identifier(c), ty.sqlite()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "CREATE TABLE IF NOT EXISTS {} ({definitions}) STRICT",
        quote_identifier(spec.name)
    )
}

fn build_insert_sql(spec: &TableSpec<'_>, rows: usize) -> String {
    let col_count = spec.columns.len();
    let row_placeholders = format!("({})", vec!["?"; col_count].join(", "));
    let mut values = String::with_capacity(rows * (col_count * 3 + 2));
    for i in 0..rows {
        if i > 0 {
            values.push(',');
        }
        values.push_str(&row_placeholders);
    }
    format!(
        "INSERT INTO {} ({}) VALUES {values}",
        quote_identifier(spec.name),
        column_list(spec.columns)
    )
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sqlx::Connection;
    use sqlx::sqlite::SqliteConnectOptions;

    use super::*;

    async fn memory_db() -> SqliteConnection {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        SqliteConnection::connect_with(&opts).await.unwrap()
    }

    fn strings(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|s| s.to_string()).collect()
    }

    fn rows(raw: &[&[&str]]) -> impl Iterator<Item = Result<Vec<String>>> {
        raw.iter()
            .map(|r| Ok(strings(r)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    async fn load(
        conn: &mut SqliteConnection,
        spec: &TableSpec<'_>,
        batch_size: u32,
        strict: bool,
        data: &[&[&str]],
    ) -> Result<u64> {
        init_insert_process(conn, spec, batch_size, strict, rows(data), None).await
    }

    #[test]
    fn identifiers_are_quoted_in_every_statement() {
        let columns = strings(&["a\"b", "c"]);
        let spec = TableSpec {
            name: "t\"x",
            columns: &columns,
            types: &[ColType::Int, ColType::Text],
        };
        assert_eq!(
            typed_table_sql(&spec),
            "CREATE TABLE IF NOT EXISTS \"t\"\"x\" (\"a\"\"b\" INTEGER, \"c\" TEXT) STRICT"
        );
        assert_eq!(
            build_insert_sql(&spec, 2),
            "INSERT INTO \"t\"\"x\" (\"a\"\"b\", \"c\") VALUES (?, ?),(?, ?)"
        );
        assert!(fts5_table_sql(&spec).starts_with("CREATE VIRTUAL TABLE IF NOT EXISTS \"t\"\"x\""));
    }

    // id, price, active, zip, typeof(price)
    type ItemRow = (Option<i64>, Option<f64>, Option<i64>, String, String);

    #[tokio::test]
    async fn typed_insert_stores_native_values_and_nulls_mismatches() {
        let mut conn = memory_db().await;
        let columns = strings(&["id", "price", "active", "zip"]);
        let spec = TableSpec {
            name: "items",
            columns: &columns,
            types: &[ColType::Int, ColType::Float, ColType::Bool, ColType::Text],
        };
        create_typed_table(&mut conn, &spec).await.unwrap();

        let data: &[&[&str]] = &[
            &["1", "9.5", "true", "007"],
            &["2", "", "FALSE", ""],
            &["three", "1e3", "maybe", "x"],
        ];
        let mismatched = load(&mut conn, &spec, 2, false, data).await.unwrap();
        assert_eq!(mismatched, 2);

        let got: Vec<ItemRow> = sqlx::query_as(
            "SELECT id, price, active, zip, typeof(price) FROM items ORDER BY rowid",
        )
        .fetch_all(&mut conn)
        .await
        .unwrap();
        assert_eq!(
            got,
            vec![
                (Some(1), Some(9.5), Some(1), "007".into(), "real".into()),
                (Some(2), None, Some(0), "".into(), "null".into()),
                (None, Some(1000.0), None, "x".into(), "real".into()),
            ]
        );
    }

    #[tokio::test]
    async fn strict_mode_rejects_a_mismatch_without_leaking_the_value() {
        let mut conn = memory_db().await;
        let columns = strings(&["n"]);
        let spec = TableSpec {
            name: "t",
            columns: &columns,
            types: &[ColType::Int],
        };
        create_typed_table(&mut conn, &spec).await.unwrap();

        let err = load(&mut conn, &spec, 10, true, &[&["1"], &["secret"]])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("inserted row 2"), "{err}");
        assert!(err.contains("'n'"), "{err}");
        assert!(!err.contains("secret"), "value leaked: {err}");
    }

    #[tokio::test]
    async fn fts_insert_keeps_every_value_as_text() {
        let mut conn = memory_db().await;
        let columns = strings(&["name", "note"]);
        let spec = TableSpec {
            name: "docs",
            columns: &columns,
            types: &[ColType::Text, ColType::Text],
        };
        create_fts5_table(&mut conn, &spec).await.unwrap();

        let data: &[&[&str]] = &[&["john smith", "42"], &["jane", ""]];
        assert_eq!(load(&mut conn, &spec, 1, true, data).await.unwrap(), 0);

        let hit: (String, String) =
            sqlx::query_as("SELECT name, typeof(note) FROM docs WHERE docs MATCH 'smith'")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(hit, ("john smith".into(), "text".into()));
    }

    #[tokio::test]
    async fn odd_table_and_column_names_round_trip() {
        let mut conn = memory_db().await;
        let columns = strings(&["first name", "a\"b"]);
        let spec = TableSpec {
            name: "my-table",
            columns: &columns,
            types: &[ColType::Text, ColType::Int],
        };
        create_typed_table(&mut conn, &spec).await.unwrap();
        load(&mut conn, &spec, 5, false, &[&["x", "5"]])
            .await
            .unwrap();

        let n: (i64,) = sqlx::query_as("SELECT \"a\"\"b\" FROM \"my-table\"")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(n.0, 5);

        delete_exists_table(&mut conn, "my-table").await.unwrap();
        let left: (i64,) =
            sqlx::query_as("SELECT count(*) FROM sqlite_master WHERE name = 'my-table'")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(left.0, 0);
    }

    #[tokio::test]
    async fn batches_split_under_the_parameter_cap() {
        let mut conn = memory_db().await;
        let columns: Vec<String> = (0..600).map(|i| format!("c{i}")).collect();
        let types = vec![ColType::Int; columns.len()];
        let spec = TableSpec {
            name: "wide",
            columns: &columns,
            types: &types,
        };
        create_typed_table(&mut conn, &spec).await.unwrap();

        let row: Vec<String> = (0..600).map(|i| i.to_string()).collect();
        let data = (0..200).map(|_| Ok(row.clone()));
        init_insert_process(&mut conn, &spec, 50_000, false, data, None)
            .await
            .unwrap();

        let n: (i64, i64) = sqlx::query_as("SELECT count(*), sum(c599) FROM wide")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(n, (200, 200 * 599));
    }

    #[tokio::test]
    async fn strict_table_rejects_wrong_storage_class() {
        // Guards the DDL: a STRICT table must refuse text in an INTEGER column
        // even when inserted outside DBMorph.
        let mut conn = memory_db().await;
        let columns = strings(&["n"]);
        let spec = TableSpec {
            name: "t",
            columns: &columns,
            types: &[ColType::Int],
        };
        create_typed_table(&mut conn, &spec).await.unwrap();
        let res = sqlx::query("INSERT INTO t (n) VALUES ('abc')")
            .execute(&mut conn)
            .await;
        assert!(res.is_err());
    }
}

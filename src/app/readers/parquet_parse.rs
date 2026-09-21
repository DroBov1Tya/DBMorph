use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use arrow::array::{Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::{ParquetRecordBatchReaderBuilder, RowSelection, RowSelector};

use crate::config;

pub fn schema_columns<P: AsRef<Path>>(path: P) -> Result<(Vec<String>, usize)> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open parquet {path:?}"))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;

    let names = builder
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().to_string())
        .collect();
    let rows = builder.metadata().file_metadata().num_rows().max(0) as usize;

    Ok((names, rows))
}

pub fn parquet_row_reader<P: AsRef<Path>>(
    path: P,
) -> Result<impl Iterator<Item = Result<Vec<String>>>> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open parquet {path:?}"))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(config::PARQUET_READ_BATCH)
        .build()?;

    let iter = reader.flat_map(|batch| match batch {
        Ok(batch) => batch_to_rows(&batch)
            .into_iter()
            .map(Ok)
            .collect::<Vec<_>>(),
        Err(e) => vec![Err(anyhow::Error::from(e))],
    });

    Ok(iter)
}

pub fn read_rows<P: AsRef<Path>>(path: P, indices: &[i64]) -> Result<Vec<Vec<String>>> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open parquet {path:?}"))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let total = builder.metadata().file_metadata().num_rows();

    let selection = selection_for(indices, total);
    let reader = builder
        .with_row_selection(selection)
        .with_batch_size(1)
        .build()?;

    let mut out = Vec::with_capacity(indices.len());
    for batch in reader {
        out.extend(batch_to_rows(&batch?));
    }

    Ok(out)
}

pub fn batch_to_rows(batch: &RecordBatch) -> Vec<Vec<String>> {
    let num_rows = batch.num_rows();
    let num_cols = batch.num_columns();

    let mut columns: Vec<StringArray> = Vec::with_capacity(num_cols);
    for i in 0..num_cols {
        let col = batch.column(i);
        let as_str = if col.data_type() == &DataType::Utf8 {
            col.as_any().downcast_ref::<StringArray>().cloned()
        } else {
            cast(col, &DataType::Utf8)
                .ok()
                .and_then(|arr| arr.as_any().downcast_ref::<StringArray>().cloned())
        };
        columns.push(as_str.unwrap_or_else(|| StringArray::from(vec![None::<&str>; num_rows])));
    }

    let mut rows = Vec::with_capacity(num_rows);
    for r in 0..num_rows {
        let mut row = Vec::with_capacity(num_cols);
        for col in &columns {
            if col.is_null(r) {
                row.push(String::new());
            } else {
                row.push(col.value(r).to_string());
            }
        }
        rows.push(row);
    }

    rows
}

fn selection_for(indices: &[i64], total: i64) -> RowSelection {
    let mut selectors = Vec::new();
    let mut cursor = 0i64;

    for &idx in indices {
        if idx < cursor || idx >= total {
            continue;
        }
        if idx > cursor {
            selectors.push(RowSelector::skip((idx - cursor) as usize));
        }
        selectors.push(RowSelector::select(1));
        cursor = idx + 1;
    }

    RowSelection::from(selectors)
}

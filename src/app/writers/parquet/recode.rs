// Copies a Parquet file into a new Parquet file with another compression.
// Data is not converted to text, so column types, NULLs, nested columns,
// file metadata and row groups stay the same.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use arrow::datatypes::{Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::basic::Compression as ParquetCompression;
use parquet::file::properties::WriterProperties;

use crate::app::readers;
use crate::app::transform;
use crate::app::utils::cli_prompt::process_and_pause;
use crate::app::utils::ui;
use crate::args::AppArgs;
use crate::config;

pub(super) async fn run(args: &AppArgs, out_path: &str) -> Result<()> {
    let (source_names, total_rows) = readers::parquet_parse::schema_columns(&args.input_path)?;
    ui::field("rows", &total_rows.to_string());
    ui::field("columns", &source_names.len().to_string());

    // Without --columns the source names are kept exactly as they are.
    let rename = args
        .columns
        .as_deref()
        .map(|spec| transform::columns::resolve_output_names(&source_names, Some(spec)))
        .transpose()?;

    let head: Vec<i64> = (0..config::PREVIEW_ROWS.min(total_rows) as i64).collect();
    let preview = readers::parquet_parse::read_rows(&args.input_path, &head)?;
    if !preview.is_empty() {
        process_and_pause(preview).await?;
    }

    let compression = super::map_compression(args.compression, args.level)?;
    ui::step(&format!(
        "re-encoding {} -> {out_path} (types and row groups preserved)",
        super::codec_label(args)
    ));

    let started = Instant::now();
    let written = recode(
        Path::new(&args.input_path),
        Path::new(out_path),
        compression,
        rename.as_deref(),
        |rows| ui::progress("write", rows, Some(total_rows as u64)),
    )?;
    ui::progress_done("wrote", written, &format!("{:.2?}", started.elapsed()));
    Ok(())
}

// Copies input into output with the given compression, one row group at a
// time. If rename is set, it gives new names for the top-level columns.
// Returns the number of rows written.
fn recode(
    input: &Path,
    output: &Path,
    compression: ParquetCompression,
    rename: Option<&[String]>,
    mut on_progress: impl FnMut(u64),
) -> Result<u64> {
    let file = File::open(input).with_context(|| format!("failed to open parquet {input:?}"))?;
    let source = ArrowReaderMetadata::load(&file, ArrowReaderOptions::default())
        .with_context(|| format!("failed to read parquet footer of {input:?}"))?;
    let file_meta = source.metadata().file_metadata();
    let expected_rows = file_meta.num_rows().max(0) as u64;

    let schema = output_schema(source.schema(), rename)?;
    let props = WriterProperties::builder()
        .set_compression(compression)
        // No row limit here, each row group is closed by the flush below.
        .set_max_row_group_row_count(None)
        // Copy the footer keys as is, other tools read them from the footer.
        .set_key_value_metadata(file_meta.key_value_metadata().cloned())
        .build();

    let sink = File::create(output).with_context(|| format!("failed to create {output:?}"))?;
    let sink = BufWriter::with_capacity(config::WRITE_BUFFER_BYTES, sink);
    let mut writer = ArrowWriter::try_new(sink, schema.clone(), Some(props))?;

    let mut written = 0u64;
    for group in 0..source.metadata().num_row_groups() {
        let batches =
            ParquetRecordBatchReaderBuilder::new_with_metadata(file.try_clone()?, source.clone())
                .with_row_groups(vec![group])
                .with_batch_size(config::PARQUET_READ_BATCH)
                .build()?;

        for batch in batches {
            let batch = batch?;
            written += batch.num_rows() as u64;
            writer.write(&RecordBatch::try_new(
                schema.clone(),
                batch.columns().to_vec(),
            )?)?;
        }
        writer.flush()?;
        on_progress(written);
    }
    writer.close()?;

    if written != expected_rows {
        bail!("re-encoded {written} row(s) but the source footer declares {expected_rows}");
    }
    Ok(written)
}

fn output_schema(source: &SchemaRef, rename: Option<&[String]>) -> Result<SchemaRef> {
    let Some(names) = rename else {
        return Ok(source.clone());
    };
    if names.len() != source.fields().len() {
        bail!(
            "{} column name(s) given but the source has {} column(s)",
            names.len(),
            source.fields().len()
        );
    }

    let fields: Vec<Field> = source
        .fields()
        .iter()
        .zip(names)
        .map(|(field, name)| field.as_ref().clone().with_name(name))
        .collect();
    Ok(Arc::new(Schema::new_with_metadata(
        fields,
        source.metadata().clone(),
    )))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use arrow::array::{
        ArrayRef, Int64Array, ListArray, StringArray, TimestampMicrosecondArray, new_null_array,
    };
    use arrow::compute::concat_batches;
    use arrow::datatypes::{DataType, Int32Type, TimeUnit};
    use parquet::basic::ZstdLevel;
    use parquet::file::metadata::{KeyValue, ParquetMetaData};

    use super::*;

    const SOURCE_GROUP_ROWS: usize = 3;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("dbmorph-recode-{}-{test}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn typed_batch() -> RecordBatch {
        let tags = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
            Some(vec![Some(1), None]),
            None,
            Some(vec![]),
            Some(vec![Some(4)]),
            Some(vec![Some(5), Some(6)]),
        ]);
        let columns: Vec<(&str, ArrayRef)> = vec![
            (
                "id",
                Arc::new(Int64Array::from(vec![
                    Some(1),
                    None,
                    Some(3),
                    Some(4),
                    Some(5),
                ])),
            ),
            (
                "name",
                Arc::new(StringArray::from(vec![
                    Some("a"),
                    Some(""),
                    None,
                    Some("d"),
                    Some("e"),
                ])),
            ),
            (
                "seen_at",
                Arc::new(TimestampMicrosecondArray::from(vec![
                    Some(1_700_000_000_000_000),
                    None,
                    Some(0),
                    Some(1),
                    Some(2),
                ])),
            ),
            ("tags", Arc::new(tags)),
        ];
        RecordBatch::try_from_iter_with_nullable(
            columns.into_iter().map(|(name, col)| (name, col, true)),
        )
        .unwrap()
    }

    fn write_source(path: &Path, batch: &RecordBatch) {
        let props = WriterProperties::builder()
            .set_compression(ParquetCompression::SNAPPY)
            .set_max_row_group_row_count(Some(SOURCE_GROUP_ROWS))
            .set_key_value_metadata(Some(vec![KeyValue::new(
                "origin".to_string(),
                "unit-test".to_string(),
            )]))
            .build();
        let mut writer =
            ArrowWriter::try_new(File::create(path).unwrap(), batch.schema(), Some(props)).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }

    fn read_all(path: &Path) -> (RecordBatch, Arc<ParquetMetaData>) {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
        let metadata = builder.metadata().clone();
        let schema = builder.schema().clone();
        let batches: Vec<RecordBatch> = builder.build().unwrap().map(Result::unwrap).collect();
        (concat_batches(&schema, &batches).unwrap(), metadata)
    }

    fn zstd(level: i32) -> ParquetCompression {
        ParquetCompression::ZSTD(ZstdLevel::try_new(level).unwrap())
    }

    fn group_rows(metadata: &ParquetMetaData) -> Vec<i64> {
        metadata.row_groups().iter().map(|g| g.num_rows()).collect()
    }

    #[test]
    fn keeps_data_schema_metadata_and_row_groups_while_changing_codec() {
        let scratch = Scratch::new("roundtrip");
        let (src, dst) = (scratch.path("in.parquet"), scratch.path("out.parquet"));
        write_source(&src, &typed_batch());

        let written = recode(&src, &dst, zstd(19), None, |_| {}).unwrap();

        let (before, before_meta) = read_all(&src);
        let (after, after_meta) = read_all(&dst);
        assert_eq!(written, 5);
        assert_eq!(after.schema(), before.schema());
        assert_eq!(after, before);
        assert_eq!(group_rows(&after_meta), vec![3, 2]);
        assert_eq!(group_rows(&after_meta), group_rows(&before_meta));

        let footer_keys = after_meta.file_metadata().key_value_metadata().unwrap();
        assert!(
            footer_keys
                .iter()
                .any(|kv| kv.key == "origin" && kv.value.as_deref() == Some("unit-test"))
        );
        for group in after_meta.row_groups() {
            for column in group.columns() {
                assert!(matches!(column.compression(), ParquetCompression::ZSTD(_)));
            }
        }
    }

    #[test]
    fn renames_top_level_columns_without_touching_values() {
        let scratch = Scratch::new("rename");
        let (src, dst) = (scratch.path("in.parquet"), scratch.path("out.parquet"));
        write_source(&src, &typed_batch());
        let names: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();

        recode(&src, &dst, zstd(3), Some(&names), |_| {}).unwrap();

        let (before, _) = read_all(&src);
        let (after, _) = read_all(&dst);
        let after_names: Vec<&str> = after
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(after_names, ["a", "b", "c", "d"]);
        assert_eq!(after.columns(), before.columns());
    }

    #[test]
    fn rejects_rename_with_wrong_column_count() {
        let scratch = Scratch::new("rename-count");
        let (src, dst) = (scratch.path("in.parquet"), scratch.path("out.parquet"));
        write_source(&src, &typed_batch());

        let err = recode(&src, &dst, zstd(3), Some(&["only".to_string()]), |_| {}).unwrap_err();

        assert!(err.to_string().contains("1 column name(s)"));
        assert!(!dst.exists(), "output must not be created on a bad rename");
    }

    #[test]
    fn recodes_file_without_rows() {
        let scratch = Scratch::new("empty");
        let (src, dst) = (scratch.path("in.parquet"), scratch.path("out.parquet"));
        let schema = typed_batch().schema();
        let empty = RecordBatch::try_new(
            schema.clone(),
            schema
                .fields()
                .iter()
                .map(|f| new_null_array(f.data_type(), 0))
                .collect(),
        )
        .unwrap();
        write_source(&src, &empty);

        let written = recode(&src, &dst, zstd(3), None, |_| {}).unwrap();

        let (after, _) = read_all(&dst);
        assert_eq!(written, 0);
        assert_eq!(after.num_rows(), 0);
        assert_eq!(after.schema().fields(), schema.fields());
        assert_eq!(
            after.schema().field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    #[test]
    fn rejects_input_that_is_not_parquet() {
        let scratch = Scratch::new("garbage");
        let (src, dst) = (scratch.path("in.parquet"), scratch.path("out.parquet"));
        std::fs::write(&src, b"id,name\n1,a\n").unwrap();

        assert!(recode(&src, &dst, zstd(3), None, |_| {}).is_err());
        assert!(!dst.exists(), "output must not be created for a bad input");
    }

    #[test]
    fn refuses_to_overwrite_its_own_input() {
        let scratch = Scratch::new("same-file");
        let src = scratch.path("in.parquet");
        write_source(&src, &typed_batch());
        let alias = scratch.path("./in.parquet");

        assert!(super::super::ensure_distinct_paths(&src, &alias).is_err());
        assert!(super::super::ensure_distinct_paths(&src, &scratch.path("out.parquet")).is_ok());
    }
}

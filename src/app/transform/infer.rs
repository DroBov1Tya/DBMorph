use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow::datatypes::DataType;

/// Inferred logical type for a Parquet column.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColType {
    Int,
    Float,
    Bool,
    Text,
}

impl ColType {
    pub fn arrow(self) -> DataType {
        match self {
            ColType::Int => DataType::Int64,
            ColType::Float => DataType::Float64,
            ColType::Bool => DataType::Boolean,
            ColType::Text => DataType::Utf8,
        }
    }
}

fn is_int(s: &str) -> bool {
    s.parse::<i64>().is_ok()
}

fn is_float(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

fn as_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Infers a type per column from a sample; empty cells count as null.
pub fn infer_types(sample: &[Vec<String>], col_count: usize) -> Vec<ColType> {
    let mut types = vec![ColType::Text; col_count];

    for (col, ty) in types.iter_mut().enumerate() {
        let mut seen = false;
        let mut all_int = true;
        let mut all_float = true;
        let mut all_bool = true;

        for row in sample {
            let Some(cell) = row.get(col) else { continue };
            if cell.is_empty() {
                continue;
            }
            seen = true;
            if !is_int(cell) {
                all_int = false;
            }
            if !is_float(cell) {
                all_float = false;
            }
            if as_bool(cell).is_none() {
                all_bool = false;
            }
        }

        *ty = if !seen {
            ColType::Text
        } else if all_bool {
            ColType::Bool
        } else if all_int {
            ColType::Int
        } else if all_float {
            ColType::Float
        } else {
            ColType::Text
        };
    }

    types
}

/// Builds a typed Arrow array from string cells; empty or unparseable => null.
pub fn build_array(ty: ColType, values: Vec<Option<String>>) -> ArrayRef {
    match ty {
        ColType::Int => {
            let arr: Int64Array = values
                .into_iter()
                .map(|v| {
                    v.filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<i64>().ok())
                })
                .collect();
            Arc::new(arr) as ArrayRef
        }
        ColType::Float => {
            let arr: Float64Array = values
                .into_iter()
                .map(|v| {
                    v.filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<f64>().ok())
                })
                .collect();
            Arc::new(arr) as ArrayRef
        }
        ColType::Bool => {
            let arr: BooleanArray = values
                .into_iter()
                .map(|v| v.filter(|s| !s.is_empty()).and_then(|s| as_bool(&s)))
                .collect();
            Arc::new(arr) as ArrayRef
        }
        ColType::Text => {
            let arr: StringArray = values.into_iter().collect();
            Arc::new(arr) as ArrayRef
        }
    }
}

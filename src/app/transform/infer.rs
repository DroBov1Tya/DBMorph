use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow::datatypes::DataType;

use crate::app::utils::ui;
use crate::config;

// Integers beyond 2^53 lose digits as f64, so they never count as floats.
const F64_EXACT_INT: u64 = 1 << 53;

// Any decimal with at most this many significant digits survives a round trip
// through f64 (DBL_DIG); longer ones would be silently rounded.
const F64_EXACT_DIGITS: usize = 15;

// Inferred logical type for a column.
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

    // Column type of a SQLite STRICT table; booleans are stored as 0/1.
    pub fn sqlite(self) -> &'static str {
        match self {
            ColType::Int | ColType::Bool => "INTEGER",
            ColType::Float => "REAL",
            ColType::Text => "TEXT",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColType::Int => "int",
            ColType::Float => "float",
            ColType::Bool => "bool",
            ColType::Text => "text",
        }
    }
}

// One cell converted to its column type.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Cell<'a> {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(&'a str),
}

// Converts a cell to its column type. Text is kept as is; an empty cell in a
// typed column is NULL. None means the value does not fit the type.
pub fn coerce(ty: ColType, s: &str) -> Option<Cell<'_>> {
    match ty {
        ColType::Text => Some(Cell::Text(s)),
        _ if s.is_empty() => Some(Cell::Null),
        ColType::Int => parse_int(s).map(Cell::Int),
        ColType::Float => parse_float(s).map(Cell::Float),
        ColType::Bool => parse_bool(s).map(Cell::Bool),
    }
}

// Digits without a redundant leading zero: "0", "42", but not "007".
fn is_canonical_digits(digits: &str) -> bool {
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

// Canonical decimal integers only, so converting never changes how a value
// reads: "007", "+5", "-0" and "1_000" stay text (zip codes, phone numbers).
fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if s == "-0" || !is_canonical_digits(digits) {
        return None;
    }
    s.parse().ok()
}

// Plain or scientific decimal notation: "-1.5", "0.25", "6.02e23". Words that
// Rust would also parse ("inf", "NaN"), ".5", "5.", and anything f64 cannot
// hold exactly (overflow, underflow, more than 15 significant digits) stay text.
fn parse_float(s: &str) -> Option<f64> {
    let (mantissa, exponent) = match s.find(['e', 'E']) {
        Some(at) => (&s[..at], Some(&s[at + 1..])),
        None => (s, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };

    if fraction.is_none() && exponent.is_none() {
        return parse_int(s)
            .filter(|n| n.unsigned_abs() <= F64_EXACT_INT)
            .map(|n| n as f64);
    }

    let whole_ok = is_canonical_digits(whole.strip_prefix('-').unwrap_or(whole));
    let fraction_ok = fraction.is_none_or(is_digits);
    let exponent_ok = exponent
        .map(|e| e.strip_prefix(['+', '-']).unwrap_or(e))
        .is_none_or(is_digits);
    if !(whole_ok && fraction_ok && exponent_ok) {
        return None;
    }

    let digits = format!(
        "{}{}",
        whole.trim_start_matches('-'),
        fraction.unwrap_or("")
    );
    let significant = digits.trim_start_matches('0').trim_end_matches('0');
    if significant.len() > F64_EXACT_DIGITS {
        return None;
    }

    let value: f64 = s.parse().ok()?;
    let representable = value.is_finite() && (value.is_normal() || significant.is_empty());
    representable.then_some(value)
}

fn parse_bool(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("true") {
        Some(true)
    } else if s.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

// Infers a type per column from a sample; empty cells count as null. A type
// is picked only when every non-empty sampled cell converts to it, so the
// sample itself always loads without loss.
pub fn infer_types(sample: &[Vec<String>], col_count: usize) -> Vec<ColType> {
    (0..col_count)
        .map(|col| {
            let mut cells = sample
                .iter()
                .filter_map(|row| row.get(col))
                .filter(|cell| !cell.is_empty())
                .peekable();
            if cells.peek().is_none() {
                return ColType::Text;
            }

            let mut fits = [ColType::Bool, ColType::Int, ColType::Float].map(|ty| (ty, true));
            for cell in cells {
                for (ty, fit) in fits.iter_mut() {
                    *fit = *fit && coerce(*ty, cell).is_some();
                }
                if fits.iter().all(|(_, fit)| !fit) {
                    return ColType::Text;
                }
            }

            fits.iter()
                .find(|(_, fit)| *fit)
                .map_or(ColType::Text, |(ty, _)| *ty)
        })
        .collect()
}

// Short summary like "3 int, 1 float, 0 bool, 2 text".
pub fn describe_types(types: &[ColType]) -> String {
    [ColType::Int, ColType::Float, ColType::Bool, ColType::Text]
        .iter()
        .map(|ty| {
            let n = types.iter().filter(|t| *t == ty).count();
            format!("{n} {}", ty.label())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// Cells that did not fit their inferred column type and were stored as NULL,
// counted per column for the closing report and the exit code.
pub struct Mismatches {
    per_column: Vec<u64>,
}

impl Mismatches {
    pub fn new(col_count: usize) -> Self {
        Self {
            per_column: vec![0; col_count],
        }
    }

    pub fn add(&mut self, col: usize, n: u64) {
        self.per_column[col] += n;
    }

    pub fn total(&self) -> u64 {
        self.per_column.iter().sum()
    }

    // Prints one warning per affected column (values are never printed, input
    // may hold personal data) and returns the total.
    pub fn finish(&self, names: &[String], types: &[ColType]) -> u64 {
        let affected: Vec<usize> = (0..self.per_column.len())
            .filter(|&col| self.per_column[col] > 0)
            .collect();

        for &col in affected.iter().take(config::SKIP_WARN_LIMIT as usize) {
            ui::warn(&format!(
                "column '{}': {} value(s) did not fit {} - stored as NULL",
                names[col],
                self.per_column[col],
                types[col].label()
            ));
        }
        if affected.len() > config::SKIP_WARN_LIMIT as usize {
            ui::warn(&format!(
                "{} more column(s) with type mismatches",
                affected.len() - config::SKIP_WARN_LIMIT as usize
            ));
        }

        let total = self.total();
        if total > 0 {
            ui::warn("raise --infer-rows or keep the columns as text to avoid NULLs");
        }
        total
    }
}

// Builds a typed Arrow array from string cells; empty cells become null.
// Also returns how many non-empty cells did not fit the type (stored as null).
pub fn build_array(ty: ColType, values: Vec<Option<String>>) -> (ArrayRef, u64) {
    let mut mismatched = 0u64;
    let arr: ArrayRef = match ty {
        ColType::Int => Arc::new(
            values
                .iter()
                .map(|v| match typed_cell(ty, v, &mut mismatched) {
                    Cell::Int(n) => Some(n),
                    _ => None,
                })
                .collect::<Int64Array>(),
        ),
        ColType::Float => Arc::new(
            values
                .iter()
                .map(|v| match typed_cell(ty, v, &mut mismatched) {
                    Cell::Float(f) => Some(f),
                    _ => None,
                })
                .collect::<Float64Array>(),
        ),
        ColType::Bool => Arc::new(
            values
                .iter()
                .map(|v| match typed_cell(ty, v, &mut mismatched) {
                    Cell::Bool(b) => Some(b),
                    _ => None,
                })
                .collect::<BooleanArray>(),
        ),
        ColType::Text => Arc::new(values.into_iter().collect::<StringArray>()),
    };
    (arr, mismatched)
}

fn typed_cell<'a>(ty: ColType, value: &'a Option<String>, mismatched: &mut u64) -> Cell<'a> {
    match value.as_deref() {
        None => Cell::Null,
        Some(s) => coerce(ty, s).unwrap_or_else(|| {
            *mismatched += 1;
            Cell::Null
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(cells: &[&str]) -> Vec<Vec<String>> {
        cells.iter().map(|c| vec![c.to_string()]).collect()
    }

    fn infer_one(cells: &[&str]) -> ColType {
        infer_types(&rows(cells), 1)[0]
    }

    #[test]
    fn int_accepts_only_canonical_decimals() {
        for (s, want) in [
            ("0", Some(0)),
            ("42", Some(42)),
            ("-17", Some(-17)),
            ("9223372036854775807", Some(i64::MAX)),
            ("-9223372036854775808", Some(i64::MIN)),
        ] {
            assert_eq!(parse_int(s), want, "{s}");
        }
        for s in [
            "",
            "-",
            "-0",
            "007",
            "+5",
            "1_000",
            " 1",
            "1 ",
            "1.0",
            "0x1F",
            "١٢",
            "9223372036854775808",
        ] {
            assert_eq!(parse_int(s), None, "{s:?}");
        }
    }

    #[test]
    fn int_round_trips_through_text() {
        for n in [0, 1, -1, 10, 1 << 40, -(1 << 40), i64::MAX, i64::MIN] {
            assert_eq!(parse_int(&n.to_string()), Some(n));
        }
    }

    #[test]
    fn float_accepts_decimal_and_scientific_notation() {
        for (s, want) in [
            ("1.5", 1.5),
            ("-0.25", -0.25),
            ("0.0", 0.0),
            ("6.02e23", 6.02e23),
            ("1E-3", 1e-3),
            ("2e+2", 200.0),
            ("42", 42.0),
            ("9007199254740992", 9_007_199_254_740_992.0),
            ("0.0e-400", 0.0),
            ("1.500000000000000000000", 1.5),
            ("0.000000000000000000123456789012345", 1.23456789012345e-19),
            ("123456789012345.0", 123_456_789_012_345.0),
        ] {
            assert_eq!(parse_float(s), Some(want), "{s}");
        }
    }

    #[test]
    fn float_rejects_words_partial_forms_overflow_and_inexact_ints() {
        for s in [
            "9007199254740993.0",
            "0.12345678901234567",
            "1e-400",
            "2.5e-320",
            "inf",
            "-inf",
            "NaN",
            "nan",
            "infinity",
            ".5",
            "5.",
            "-.5",
            "1e",
            "1e+",
            "e5",
            "01.5",
            "1.5.2",
            "1,5",
            "1e999",
            " 1.5",
            "9007199254740993",
        ] {
            assert_eq!(parse_float(s), None, "{s:?}");
        }
    }

    #[test]
    fn bool_is_true_or_false_in_any_case() {
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("False"), Some(false));
        assert_eq!(parse_bool("1"), None);
        assert_eq!(parse_bool("yes"), None);
    }

    #[test]
    fn coerce_maps_empty_typed_cells_to_null_and_keeps_text() {
        assert_eq!(coerce(ColType::Int, ""), Some(Cell::Null));
        assert_eq!(coerce(ColType::Text, ""), Some(Cell::Text("")));
        assert_eq!(coerce(ColType::Text, "007"), Some(Cell::Text("007")));
        assert_eq!(coerce(ColType::Int, "abc"), None);
        assert_eq!(coerce(ColType::Bool, "true"), Some(Cell::Bool(true)));
    }

    #[test]
    fn infer_picks_the_narrowest_type_that_fits_every_cell() {
        assert_eq!(infer_one(&["1", "2", ""]), ColType::Int);
        assert_eq!(infer_one(&["1", "2.5"]), ColType::Float);
        assert_eq!(infer_one(&["true", "FALSE", ""]), ColType::Bool);
        assert_eq!(infer_one(&["1", "x"]), ColType::Text);
        assert_eq!(infer_one(&["007", "123"]), ColType::Text);
        assert_eq!(infer_one(&["12345678901234567890"]), ColType::Text);
        assert_eq!(infer_one(&["", ""]), ColType::Text);
        assert_eq!(infer_one(&[]), ColType::Text);
    }

    #[test]
    fn infer_handles_short_rows_and_many_columns() {
        let sample = vec![
            vec!["1".to_string(), "a".to_string(), "1.5".to_string()],
            vec!["2".to_string()],
        ];
        assert_eq!(
            infer_types(&sample, 4),
            vec![ColType::Int, ColType::Text, ColType::Float, ColType::Text]
        );
    }

    #[test]
    fn every_sampled_cell_converts_under_the_inferred_type() {
        let cells = ["0", "-3", "4.25", "", "1e3", "17"];
        let ty = infer_one(&cells);
        assert_eq!(ty, ColType::Float);
        assert!(cells.iter().all(|c| coerce(ty, c).is_some()));
    }

    #[test]
    fn build_array_counts_cells_that_do_not_fit() {
        let values = vec![
            Some("1".to_string()),
            Some("oops".to_string()),
            Some(String::new()),
            None,
        ];
        let (arr, mismatched) = build_array(ColType::Int, values);
        assert_eq!(mismatched, 1);
        let ints = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(
            ints.iter().collect::<Vec<_>>(),
            vec![Some(1), None, None, None]
        );
    }

    #[test]
    fn describe_types_counts_each_kind() {
        let types = [ColType::Int, ColType::Text, ColType::Int, ColType::Bool];
        assert_eq!(describe_types(&types), "2 int, 0 float, 1 bool, 1 text");
    }
}

use std::{error::Error, fmt};

use jaq_core::{
    Ctx, Vars,
    data::JustLut,
    load::{Arena, Error as LoadError, File, Loader},
    unwrap_valr,
};
use jaq_json::Val;
use serde_json::Value;

pub const MAX_FILTER_OUTPUTS: usize = 10_000;

#[derive(Debug, Eq, PartialEq)]
pub struct FilterOutput {
    pub value: Value,
    pub count: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FilterError(String);

impl FilterError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FilterError {}

pub fn evaluate(input: &Value, program: &str) -> Result<FilterOutput, FilterError> {
    let input = serde_json::to_vec(input)
        .map_err(|error| FilterError::new(format!("could not prepare input: {error}")))?;
    let input = jaq_json::read::parse_single(input.as_slice())
        .map_err(|error| FilterError::new(format!("could not prepare input: {error}")))?;

    let arena = Arena::default();
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let loader = Loader::new(defs);
    let modules = loader
        .load(
            &arena,
            File {
                code: program,
                path: (),
            },
        )
        .map_err(format_load_errors)?;
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let filter = jaq_core::Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(format_compile_errors)?;

    let context = Ctx::<JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut values = Vec::new();
    for result in filter.id.run((context, input)).map(unwrap_valr) {
        if values.len() == MAX_FILTER_OUTPUTS {
            return Err(FilterError::new(format!(
                "filter produced more than {MAX_FILTER_OUTPUTS} outputs; refine the expression"
            )));
        }
        let value = result.map_err(|error| FilterError::new(format!("runtime error: {error}")))?;
        let value = serde_json::from_str(&value.to_string()).map_err(|error| {
            FilterError::new(format!("filter produced a non-JSON value: {error}"))
        })?;
        values.push(value);
    }

    let count = values.len();
    let value = match count {
        0 => Value::Array(Vec::new()),
        1 => values.pop().expect("one filter output was collected"),
        _ => Value::Array(values),
    };
    Ok(FilterOutput { value, count })
}

fn format_load_errors(errors: jaq_core::load::Errors<&str, ()>) -> FilterError {
    let Some((_, error)) = errors.first() else {
        return FilterError::new("invalid jq filter");
    };
    let detail = match error {
        LoadError::Io(errors) => errors
            .first()
            .map(|(path, error)| format!("could not load {path:?}: {error}"))
            .unwrap_or_else(|| "could not load module".into()),
        LoadError::Lex(errors) => errors
            .first()
            .map(|(expected, found)| {
                format!(
                    "expected {}, found {}",
                    expected.as_str(),
                    found_preview(found)
                )
            })
            .unwrap_or_else(|| "invalid token".into()),
        LoadError::Parse(errors) => errors
            .first()
            .map(|(expected, found)| {
                let found = found_preview(found);
                format!("expected {}, found {found}", expected.as_str())
            })
            .unwrap_or_else(|| "invalid expression".into()),
    };
    FilterError::new(format!("syntax error: {detail}"))
}

fn format_compile_errors(errors: jaq_core::compile::Errors<&str, ()>) -> FilterError {
    let detail = errors
        .first()
        .and_then(|(_, errors)| errors.first())
        .map(|(name, undefined)| format!("undefined {} {name:?}", undefined.as_str()))
        .unwrap_or_else(|| "could not compile expression".into());
    FilterError::new(format!("compile error: {detail}"))
}

fn found_preview(found: &str) -> String {
    let found = found.chars().take(24).collect::<String>();
    if found.is_empty() {
        "end of expression".into()
    } else {
        format!("{found:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evaluates_standard_jq_filters() {
        let output = evaluate(
            &json!({"users": [{"name": "Ada", "active": true}, {"name": "Lin", "active": false}]}),
            ".users | map(select(.active)) | .[].name",
        )
        .unwrap();

        assert_eq!(
            output,
            FilterOutput {
                value: json!("Ada"),
                count: 1
            }
        );
    }

    #[test]
    fn wraps_multiple_outputs_for_tree_navigation() {
        let output = evaluate(&json!([1, 2, 3]), ".[] | . * 2").unwrap();

        assert_eq!(
            output,
            FilterOutput {
                value: json!([2, 4, 6]),
                count: 3
            }
        );
    }

    #[test]
    fn represents_an_empty_stream_as_an_empty_tree_array() {
        let output = evaluate(&json!([1, 2]), ".[] | select(. > 5)").unwrap();

        assert_eq!(
            output,
            FilterOutput {
                value: json!([]),
                count: 0
            }
        );
    }

    #[test]
    fn reports_syntax_and_runtime_errors() {
        assert!(
            evaluate(&json!(null), ".[")
                .unwrap_err()
                .to_string()
                .contains("syntax error")
        );
        assert!(
            evaluate(&json!(1), ".[]")
                .unwrap_err()
                .to_string()
                .contains("runtime error")
        );
    }
}

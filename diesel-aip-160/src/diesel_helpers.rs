//! Reusable PostgreSQL Diesel predicates for filter adapters.

use anyhow::{Result, bail, ensure};
#[cfg(feature = "postgres")]
use diesel::PgJsonbExpressionMethods;
use diesel::expression::{ValidGrouping, is_aggregate::No};
#[cfg(all(feature = "mysql", not(feature = "postgres")))]
use diesel::mysql::Mysql as Backend;
#[cfg(feature = "postgres")]
use diesel::pg::Pg as Backend;
use diesel::prelude::*;
#[cfg(feature = "postgres")]
use diesel::sql_types::Jsonb;
use diesel::sql_types::{Bool, Integer, IntoNullable, Nullable, SqlType, Text};
#[cfg(all(feature = "sqlite", not(any(feature = "postgres", feature = "mysql"))))]
use diesel::sqlite::Sqlite as Backend;
use diesel::{BoxableExpression, Column};

use super::compiler::ComparisonOperator;

pub type Predicate<Table> = Box<dyn BoxableExpression<Table, Backend, SqlType = Bool>>;

/// Extract a JSONB value by a bound path. PostgreSQL returns SQL NULL for a
/// missing path, distinct from a present JSON null.
#[cfg(feature = "postgres")]
pub fn jsonb_presence<C>(column: C, path: Vec<String>) -> Predicate<C::Table>
where
    C: Column<SqlType = Jsonb>
        + BoxableExpression<C::Table, Backend, SqlType = Jsonb>
        + ValidGrouping<(), IsAggregate = No>
        + 'static,
{
    Box::new(column.retrieve_by_path_as_object(path).is_not_null())
}

#[cfg(feature = "postgres")]
pub fn jsonb_compare<C>(
    column: C,
    path: Vec<String>,
    operator: ComparisonOperator,
    value: serde_json::Value,
) -> Predicate<C::Table>
where
    C: Column<SqlType = Jsonb>
        + BoxableExpression<C::Table, Backend, SqlType = Jsonb>
        + ValidGrouping<(), IsAggregate = No>
        + 'static,
{
    let extracted = column.retrieve_by_path_as_object(path);
    match operator {
        ComparisonOperator::Equal => Box::new(extracted.eq(value).assume_not_null()),
        ComparisonOperator::NotEqual => Box::new(extracted.ne(value).assume_not_null()),
        ComparisonOperator::LessThan => Box::new(extracted.lt(value).assume_not_null()),
        ComparisonOperator::LessThanOrEqual => Box::new(extracted.le(value).assume_not_null()),
        ComparisonOperator::GreaterThan => Box::new(extracted.gt(value).assume_not_null()),
        ComparisonOperator::GreaterThanOrEqual => Box::new(extracted.ge(value).assume_not_null()),
    }
}

#[cfg(feature = "postgres")]
pub fn jsonb_contains<C>(
    column: C,
    path: Vec<String>,
    value: serde_json::Value,
) -> Predicate<C::Table>
where
    C: Column<SqlType = Jsonb>
        + BoxableExpression<C::Table, Backend, SqlType = Jsonb>
        + ValidGrouping<(), IsAggregate = No>
        + 'static,
{
    Box::new(
        column
            .retrieve_by_path_as_object(path)
            .contains(value)
            .assume_not_null(),
    )
}

/// Restricts the helper to text columns and preserves their schema nullability.
/// Diesel's `Varchar` is an alias of `Text`.
pub trait StringSqlType: SqlType + IntoNullable<Nullable = Nullable<Text>> {
    const NULLABLE: bool;
}

impl StringSqlType for Text {
    const NULLABLE: bool = false;
}

impl StringSqlType for Nullable<Text> {
    const NULLABLE: bool = true;
}

/// Compare either a Text or Nullable<Text> column without losing table typing.
/// `None` emits IS NULL / IS NOT NULL, and is rejected for nonnullable columns
/// or ordering operators. `Some` always binds a string, including "null".
///
/// The nullable/assume_not_null wrappers change Diesel's expression types only;
/// SQL comparisons retain PostgreSQL's existing three-valued NULL semantics.
pub fn compare_string_column<C>(
    column: C,
    operator: ComparisonOperator,
    value: Option<String>,
) -> Result<Predicate<C::Table>>
where
    C: Column + 'static,
    C::SqlType: StringSqlType,
    diesel::dsl::Nullable<C>: BoxableExpression<C::Table, Backend, SqlType = Nullable<Text>>
        + ValidGrouping<(), IsAggregate = No>,
{
    if value.is_none() {
        ensure!(C::SqlType::NULLABLE, "field `{}` is not nullable", C::NAME);
    }
    let column = column.nullable();
    Ok(match value {
        Some(value) => match operator {
            ComparisonOperator::Equal => Box::new(column.eq(value).assume_not_null()),
            ComparisonOperator::NotEqual => Box::new(column.ne(value).assume_not_null()),
            ComparisonOperator::GreaterThan => Box::new(column.gt(value).assume_not_null()),
            ComparisonOperator::GreaterThanOrEqual => Box::new(column.ge(value).assume_not_null()),
            ComparisonOperator::LessThan => Box::new(column.lt(value).assume_not_null()),
            ComparisonOperator::LessThanOrEqual => Box::new(column.le(value).assume_not_null()),
        },
        None => match operator {
            ComparisonOperator::Equal => Box::new(column.is_null()),
            ComparisonOperator::NotEqual => Box::new(column.is_not_null()),
            _ => bail!("null supports only = and != comparisons"),
        },
    })
}

/// Restricts integer comparisons to PostgreSQL Integer (i32) columns,
/// preserving schema nullability.
pub trait IntegerSqlType: SqlType + IntoNullable<Nullable = Nullable<Integer>> {
    const NULLABLE: bool;
}

impl IntegerSqlType for Integer {
    const NULLABLE: bool = false;
}

impl IntegerSqlType for Nullable<Integer> {
    const NULLABLE: bool = true;
}

/// Compare Integer or Nullable<Integer> columns using bound i32 values.
/// `None` supports only equality/inequality on nullable columns and emits
/// IS NULL / IS NOT NULL. Other comparisons preserve SQL NULL semantics.
pub fn compare_integer_column<C>(
    column: C,
    operator: ComparisonOperator,
    value: Option<i32>,
) -> Result<Predicate<C::Table>>
where
    C: Column + 'static,
    C::SqlType: IntegerSqlType,
    diesel::dsl::Nullable<C>: BoxableExpression<C::Table, Backend, SqlType = Nullable<Integer>>
        + ValidGrouping<(), IsAggregate = No>,
{
    if value.is_none() {
        ensure!(C::SqlType::NULLABLE, "field `{}` is not nullable", C::NAME);
    }
    let column = column.nullable();
    Ok(match value {
        Some(value) => match operator {
            ComparisonOperator::Equal => Box::new(column.eq(value).assume_not_null()),
            ComparisonOperator::NotEqual => Box::new(column.ne(value).assume_not_null()),
            ComparisonOperator::GreaterThan => Box::new(column.gt(value).assume_not_null()),
            ComparisonOperator::GreaterThanOrEqual => Box::new(column.ge(value).assume_not_null()),
            ComparisonOperator::LessThan => Box::new(column.lt(value).assume_not_null()),
            ComparisonOperator::LessThanOrEqual => Box::new(column.le(value).assume_not_null()),
        },
        None => match operator {
            ComparisonOperator::Equal => Box::new(column.is_null()),
            ComparisonOperator::NotEqual => Box::new(column.is_not_null()),
            _ => bail!("null supports only = and != comparisons"),
        },
    })
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    table! {
        strings (id) {
            id -> Integer,
            required -> Text,
            optional -> Nullable<Text>,
            required_integer -> Integer,
            optional_integer -> Nullable<Integer>,
        }
    }

    fn sql(predicate: Predicate<strings::table>) -> String {
        let query = strings::table.filter(predicate).select(strings::id);
        diesel::debug_query::<Backend, _>(&query).to_string()
    }

    #[test]
    fn compares_required_and_optional_text_with_bound_values() {
        for (operator, token) in [
            (ComparisonOperator::Equal, "="),
            (ComparisonOperator::NotEqual, "!="),
            (ComparisonOperator::GreaterThan, ">"),
            (ComparisonOperator::GreaterThanOrEqual, ">="),
            (ComparisonOperator::LessThan, "<"),
            (ComparisonOperator::LessThanOrEqual, "<="),
        ] {
            let required =
                compare_string_column(strings::required, operator, Some("null".into())).unwrap();
            let optional =
                compare_string_column(strings::optional, operator, Some("null".into())).unwrap();
            for (field, predicate) in [("required", required), ("optional", optional)] {
                let sql = sql(predicate);
                assert!(
                    sql.contains(&format!("\"strings\".\"{field}\" {token} $1")),
                    "{sql}"
                );
                assert!(sql.ends_with("-- binds: [\"null\"]"), "{sql}");
                assert!(
                    !sql.contains("COALESCE"),
                    "NULL comparisons retain three-valued logic"
                );
            }
        }
    }

    #[test]
    fn null_checks_do_not_bind_null_as_a_comparison_value() {
        for (operator, token) in [
            (ComparisonOperator::Equal, "IS NULL"),
            (ComparisonOperator::NotEqual, "IS NOT NULL"),
        ] {
            let predicate = compare_string_column(strings::optional, operator, None).unwrap();
            let sql = sql(predicate);
            assert!(sql.contains(&format!("\"optional\" {token}")), "{sql}");
            assert!(sql.ends_with("-- binds: []"), "{sql}");
        }
    }

    #[test]
    fn rejects_null_for_nonnullable_columns_and_ordering() {
        for operator in [ComparisonOperator::Equal, ComparisonOperator::NotEqual] {
            let error = compare_string_column(strings::required, operator, None)
                .err()
                .expect("required text cannot be null");
            assert_eq!(error.to_string(), "field `required` is not nullable");
        }
        for operator in [
            ComparisonOperator::GreaterThan,
            ComparisonOperator::GreaterThanOrEqual,
            ComparisonOperator::LessThan,
            ComparisonOperator::LessThanOrEqual,
        ] {
            let error = compare_string_column(strings::optional, operator, None)
                .err()
                .expect("null cannot be ordered");
            assert_eq!(error.to_string(), "null supports only = and != comparisons");
        }
    }

    #[test]
    fn compares_required_and_optional_integers_without_precision_loss() {
        for (operator, token) in [
            (ComparisonOperator::Equal, "="),
            (ComparisonOperator::NotEqual, "!="),
            (ComparisonOperator::GreaterThan, ">"),
            (ComparisonOperator::GreaterThanOrEqual, ">="),
            (ComparisonOperator::LessThan, "<"),
            (ComparisonOperator::LessThanOrEqual, "<="),
        ] {
            for value in [i32::MIN, -1, 0, i32::MAX] {
                let required =
                    compare_integer_column(strings::required_integer, operator, Some(value))
                        .unwrap();
                let optional =
                    compare_integer_column(strings::optional_integer, operator, Some(value))
                        .unwrap();
                for (field, predicate) in [
                    ("required_integer", required),
                    ("optional_integer", optional),
                ] {
                    let sql = sql(predicate);
                    assert!(
                        sql.contains(&format!("\"strings\".\"{field}\" {token} $1")),
                        "{sql}"
                    );
                    assert!(sql.ends_with(&format!("-- binds: [{value}]")), "{sql}");
                    assert!(
                        !sql.contains("COALESCE"),
                        "NULL comparisons retain three-valued logic"
                    );
                }
            }
        }
    }

    #[test]
    fn integer_null_checks_use_sql_null_operators() {
        for (operator, token) in [
            (ComparisonOperator::Equal, "IS NULL"),
            (ComparisonOperator::NotEqual, "IS NOT NULL"),
        ] {
            let predicate =
                compare_integer_column(strings::optional_integer, operator, None).unwrap();
            let sql = sql(predicate);
            assert!(
                sql.contains(&format!("\"optional_integer\" {token}")),
                "{sql}"
            );
            assert!(sql.ends_with("-- binds: []"), "{sql}");
        }
    }

    #[test]
    fn integer_null_checks_reject_nonnullable_columns_and_ordering() {
        for operator in [ComparisonOperator::Equal, ComparisonOperator::NotEqual] {
            let error = compare_integer_column(strings::required_integer, operator, None)
                .err()
                .expect("required integers cannot be null");
            assert_eq!(
                error.to_string(),
                "field `required_integer` is not nullable"
            );
        }
        for operator in [
            ComparisonOperator::GreaterThan,
            ComparisonOperator::GreaterThanOrEqual,
            ComparisonOperator::LessThan,
            ComparisonOperator::LessThanOrEqual,
        ] {
            let error = compare_integer_column(strings::optional_integer, operator, None)
                .err()
                .expect("null cannot be ordered");
            assert_eq!(error.to_string(), "null supports only = and != comparisons");
        }
    }
}

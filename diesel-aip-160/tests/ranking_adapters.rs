//! Generic metrics and backward-compatible adapters on every Diesel backend.
#![cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use diesel::prelude::*;
use diesel_aip_160::{
    Aip160Filter,
    compiler::QueryFilter,
    diesel_helpers::{FieldAdapter, Predicate, SearchAdapter},
    parser::Value,
    ranking::{CompiledQueryFilter, Direction, RankingSignal},
};
#[cfg(feature = "postgres")]
type Backend = diesel::pg::Pg;
#[cfg(feature = "mysql")]
type Backend = diesel::mysql::Mysql;
#[cfg(feature = "sqlite")]
type Backend = diesel::sqlite::Sqlite;

diesel::table! { places (id) { id -> Integer, label -> Text } }
diesel::define_sql_function! { fn abs(value: diesel::sql_types::Integer) -> diesel::sql_types::Integer; }
struct Distance;
impl SearchAdapter<places::table> for Distance {
    fn search(_: &Value) -> diesel_aip_160::anyhow::Result<Predicate<places::table>> {
        Ok(Box::new(places::id.ge(0)))
    }
    fn search_query(
        value: &Value,
    ) -> diesel_aip_160::anyhow::Result<CompiledQueryFilter<places::table>> {
        let center: i32 = value.text.parse()?;
        Ok(QueryFilter {
            predicate: Self::search(value)?,
            signals: vec![
                RankingSignal::new(
                    "distance",
                    abs(places::id - center),
                    Direction::Ascending,
                    "demo.distance",
                ),
                RankingSignal::new("label", places::label, Direction::Descending, "demo.label"),
            ],
        })
    }
}
struct Coordinate;
impl FieldAdapter<places::id> for Distance {
    fn compare(
        column: places::id,
        _: diesel_aip_160::compiler::ComparisonOperator,
        value: &Value,
    ) -> diesel_aip_160::anyhow::Result<Predicate<places::table>> {
        Ok(Box::new(column.ge(value.text.parse::<i32>()?)))
    }
    fn compare_query(
        column: places::id,
        operator: diesel_aip_160::compiler::ComparisonOperator,
        value: &Value,
    ) -> diesel_aip_160::anyhow::Result<CompiledQueryFilter<places::table>> {
        let mut result = <Self as SearchAdapter<places::table>>::search_query(value)?;
        result.predicate = Self::compare(column, operator, value)?;
        Ok(result)
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = places)]
#[allow(dead_code)]
struct Place {
    #[aip160(adapter = Distance, search_adapter = Distance)]
    id: Coordinate,
    label: String,
}

#[test]
fn typed_metrics_single_priority_order_and_application_tie_breaker() {
    let mut result = Place::compile_query_filter("id:7").unwrap().unwrap();
    assert_eq!(result.signals.len(), 2);
    assert_eq!(result.signals[0].direction, Direction::Ascending);
    assert_eq!(result.signals[0].operation, "demo.distance");
    assert!(result.ordering(&[]).is_err());
    assert!(result.ordering(&["missing"]).is_err());
    assert!(result.ordering(&["distance", "distance"]).is_err());
    assert_eq!(result.signals.len(), 2); // Invalid policies consume nothing.
    let order = result.ordering(&["distance", "label"]).unwrap();
    let query = places::table
        .filter(result.predicate)
        .order(order)
        .then_order_by(places::id)
        .limit(2)
        .offset(1);
    let sql = diesel::debug_query::<Backend, _>(&query).to_string();
    assert!(
        sql.contains("abs((\"places\".\"id\" - ") || sql.contains("abs((`places`.`id` - "),
        "{sql}"
    );
    assert!(sql.contains(" ASC, ") && sql.contains(" DESC, "), "{sql}");
    assert!(sql.find("ORDER BY").unwrap() < sql.find("LIMIT").unwrap());
    let (statement, binds) = sql.split_once("-- binds:").unwrap();
    assert!(!statement.contains('7'), "{sql}");
    assert!(binds.contains('7'), "{sql}");
    let mut result = Place::compile_query_filter("id:7").unwrap().unwrap();
    let expression = result
        .take_signal("distance")
        .unwrap()
        .into_expression::<diesel::sql_types::Integer>()
        .unwrap();
    let query = places::table
        .filter(result.predicate)
        .order(expression.asc())
        .then_order_by(places::id);
    assert!(
        diesel::debug_query::<Backend, _>(&query)
            .to_string()
            .contains(" ASC")
    );
    let mut result = Place::compile_query_filter("id:7").unwrap().unwrap();
    assert!(
        result
            .take_signal("label")
            .unwrap()
            .into_expression::<diesel::sql_types::Float>()
            .is_err()
    );
    assert!(
        Place::compile_query_filter("id=7")
            .unwrap()
            .unwrap()
            .take_signal("distance")
            .is_ok()
    );
}

#[test]
fn generic_boolean_composition_does_not_invent_text_scoring_rules() {
    for (source, count) in [
        ("NOT id:7", 0),
        ("NOT (NOT id:7)", 2),
        ("id:(7 OR 8)", 4),
        ("id:7 AND NOT id:8", 2),
        ("NOT (id:7 OR NOT id:8)", 2),
    ] {
        let result = Place::compile_query_filter(source).unwrap().unwrap();
        assert_eq!(result.signals.len(), count, "{source}");
    }
    // Generic metrics remain independent. Repeated names require application
    // disambiguation; text's aggregation/deduplication is not imposed on them.
    let mut result = Place::compile_query_filter("id:7 OR id:7")
        .unwrap()
        .unwrap();
    assert!(result.ordering(&["distance"]).is_err());
    assert_eq!(result.signals.len(), 4);
    assert!(Place::compile_query_filter("").unwrap().is_none());
    let result = Place::compile_query_filter("label=home").unwrap().unwrap();
    assert!(result.signals.is_empty());
    let legacy = Place::compile_filter("id:7").unwrap().unwrap();
    let ranked = Place::compile_query_filter("id:7").unwrap().unwrap();
    assert_eq!(
        diesel::debug_query::<Backend, _>(&places::table.filter(legacy)).to_string(),
        diesel::debug_query::<Backend, _>(&places::table.filter(ranked.predicate)).to_string()
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn generic_distance_orders_before_pagination_in_sqlite() {
    use diesel::connection::SimpleConnection;
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("CREATE TABLE places (id integer primary key, label text not null); INSERT INTO places VALUES (1,'a'),(5,'b'),(7,'c'),(9,'d');").unwrap();
    let mut result = Place::compile_query_filter("id:6").unwrap().unwrap();
    let order = result.ordering(&["distance"]).unwrap();
    let ids = places::table
        .filter(result.predicate)
        .select(places::id)
        .order(order)
        .then_order_by(places::id)
        .limit(2)
        .offset(1)
        .load::<i32>(&mut conn)
        .unwrap();
    assert_eq!(ids, vec![7, 9]);
    let mut result = Place::compile_query_filter("id:6").unwrap().unwrap();
    let order = result.ordering(&["distance", "label"]).unwrap();
    let ids = places::table
        .filter(result.predicate)
        .select(places::id)
        .order(order)
        .then_order_by(places::id)
        .limit(2)
        .load::<i32>(&mut conn)
        .unwrap();
    assert_eq!(ids, vec![7, 5]);
}

#[test]
fn signals_accept_bound_constants_and_preserve_sql_type_metadata() {
    use diesel::IntoSql;
    let signal: RankingSignal<places::table> = RankingSignal::new(
        "constant",
        1.5_f64.into_sql::<diesel::sql_types::Double>(),
        Direction::Descending,
        "demo.constant",
    );
    assert_eq!(
        signal.sql_type(),
        std::any::type_name::<diesel::sql_types::Double>()
    );
    let expression = signal
        .into_expression::<diesel::sql_types::Double>()
        .unwrap();
    let query = places::table.select(expression).order(places::id);
    let sql = diesel::debug_query::<Backend, _>(&query).to_string();
    let (statement, binds) = sql.split_once("-- binds:").unwrap();
    assert!(!statement.contains("1.5"));
    assert!(binds.contains("1.5"));
}

//! Native type conformance: compile separately for each backend.
#![cfg(all(feature = "sqlite", feature = "native-types"))]
#[cfg(feature = "sqlite")]
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel_aip_160::Aip160Filter;
diesel::table! { native_types (id) { id -> Integer,
    text_value -> Nullable<Text>,
    bool_value -> Nullable<Bool>,
    small_value -> Nullable<SmallInt>,
    integer_value -> Nullable<Integer>,
    big_value -> Nullable<BigInt>,
    float_value -> Nullable<Float>,
    double_value -> Nullable<Double>,
    numeric_value -> Nullable<Numeric>,
    binary_value -> Nullable<Binary>,
    date_value -> Nullable<Date>,
    time_value -> Nullable<Time>,
    timestamp_value -> Nullable<Timestamp>,
    json_value -> Nullable<Json>,
    jsonb_value -> Nullable<Jsonb>,
    tz_value -> Nullable<TimestamptzSqlite>,
} }

#[derive(Aip160Filter)]
#[diesel(table_name = native_types)]
#[allow(dead_code)]
struct NativeModel {
    id: i32,
    text_value: Option<String>,
    bool_value: Option<bool>,
    small_value: Option<i16>,
    integer_value: Option<i32>,
    big_value: Option<i64>,
    float_value: Option<f32>,
    double_value: Option<f64>,
    numeric_value: Option<bigdecimal::BigDecimal>,
    binary_value: Option<Vec<u8>>,
    date_value: Option<chrono::NaiveDate>,
    time_value: Option<chrono::NaiveTime>,
    timestamp_value: Option<chrono::NaiveDateTime>,
    json_value: Option<serde_json::Value>,
    jsonb_value: Option<serde_json::Value>,
    tz_value: Option<chrono::DateTime<chrono::Utc>>,
}

const CASES: &[&str] = &[
    r#"text_value="Hello""#,
    r#"bool_value=true"#,
    r#"small_value=32767"#,
    r#"integer_value=2147483647"#,
    r#"big_value=9007199254740993"#,
    r#"float_value=0.1"#,
    r#"double_value=2.5"#,
    r#"numeric_value=12345678901234567890.123456789"#,
    r#"binary_value="0xdeadbeef""#,
    r#"date_value="2026-10-17""#,
    r#"time_value="12:34:56.123456""#,
    r#"timestamp_value="2026-10-17T12:34:56.123456""#,
    r#"json_value='{"a":1}'"#,
    r#"jsonb_value='{"a":1}'"#,
    r#"tz_value="2026-10-17T12:34:56Z""#,
];

#[test]
fn native_types_compile_and_null_checks_bind_no_values() {
    for filter in CASES {
        let predicate = NativeModel::compile_filter(filter)
            .unwrap_or_else(|e| panic!("{filter}: {e}"))
            .unwrap();
        let sql = diesel::debug_query::<diesel::sqlite::Sqlite, _>(
            &native_types::table
                .filter(predicate)
                .select(native_types::id),
        )
        .to_string();
        assert!(sql.contains("-- binds: ["), "{filter}: {sql}");
        let field = filter.split('=').next().unwrap();
        for suffix in ["=null", "!=null", ":*"] {
            let predicate = NativeModel::compile_filter(&format!("{field}{suffix}"))
                .unwrap()
                .unwrap();
            let sql = diesel::debug_query::<diesel::sqlite::Sqlite, _>(
                &native_types::table
                    .filter(predicate)
                    .select(native_types::id),
            )
            .to_string();
            assert!(sql.ends_with("-- binds: []"), "{sql}");
        }
    }
    for invalid in [
        "bool_value=1",
        "bool_value='true'",
        "small_value=32768",
        "small_value=-32769",
        "float_value=1e100",
        "double_value=NaN",
        "numeric_value=oops",
        "binary_value='0x0g'",
        "date_value='2026-02-30'",
        "bool_value:yes",
        "id=null",
    ] {
        assert!(NativeModel::compile_filter(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn native_types_execute_against_database() {
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("CREATE TEMPORARY TABLE native_types (id INTEGER PRIMARY KEY, text_value Text, bool_value BOOLEAN, small_value SmallInt, integer_value Integer, big_value BigInt, float_value REAL, double_value REAL, numeric_value NUMERIC, binary_value BLOB, date_value Date, time_value Time, timestamp_value Timestamp, json_value TEXT, jsonb_value BLOB, tz_value TEXT); INSERT INTO native_types VALUES (1,'Hello',true,32767,2147483647,9007199254740993,0.1,2.5,12345678901234567890.123456789,X'deadbeef','2026-10-17','12:34:56.123456','2026-10-17T12:34:56.123456','{\"a\":1}',jsonb('{\"a\":1}'),'2026-10-17T12:34:56Z'); INSERT INTO native_types (id) VALUES (2);").unwrap();
    diesel::update(native_types::table.filter(native_types::id.eq(1)))
        .set((
            native_types::timestamp_value.eq(chrono::NaiveDateTime::parse_from_str(
                "2026-10-17T12:34:56.123456",
                "%Y-%m-%dT%H:%M:%S%.f",
            )
            .unwrap()),
            native_types::tz_value
                .eq(chrono::DateTime::parse_from_rfc3339("2026-10-17T12:34:56Z").unwrap()),
        ))
        .execute(&mut conn)
        .unwrap();
    for filter in CASES {
        let predicate = NativeModel::compile_filter(filter).unwrap().unwrap();
        let ids = native_types::table
            .filter(predicate)
            .select(native_types::id)
            .load::<i32>(&mut conn)
            .unwrap_or_else(|e| panic!("{filter}: {e}"));
        assert_eq!(ids, vec![1], "{filter}");
        let field = filter.split('=').next().unwrap();
        for (suffix, expected) in [("=null", vec![2]), ("!=null", vec![1]), (":*", vec![1])] {
            let predicate = NativeModel::compile_filter(&format!("{field}{suffix}"))
                .unwrap()
                .unwrap();
            assert_eq!(
                native_types::table
                    .filter(predicate)
                    .select(native_types::id)
                    .load::<i32>(&mut conn)
                    .unwrap(),
                expected,
                "{field}{suffix}"
            );
        }
    }
}

diesel::table! { required_types (id) { id -> Integer,
    text_value -> Text,
    bool_value -> Bool,
    small_value -> SmallInt,
    integer_value -> Integer,
    big_value -> BigInt,
    float_value -> Float,
    double_value -> Double,
    numeric_value -> Numeric,
    binary_value -> Binary,
    date_value -> Date,
    time_value -> Time,
    timestamp_value -> Timestamp,
    json_value -> Json,
    jsonb_value -> Jsonb,
    tz_value -> TimestamptzSqlite,
} }
#[derive(Aip160Filter)]
#[diesel(table_name = required_types)]
#[allow(dead_code)]
struct RequiredModel {
    id: i32,
    text_value: String,
    bool_value: bool,
    small_value: i16,
    integer_value: i32,
    big_value: i64,
    float_value: f32,
    double_value: f64,
    numeric_value: bigdecimal::BigDecimal,
    binary_value: Vec<u8>,
    date_value: chrono::NaiveDate,
    time_value: chrono::NaiveTime,
    timestamp_value: chrono::NaiveDateTime,
    json_value: serde_json::Value,
    jsonb_value: serde_json::Value,
    tz_value: chrono::DateTime<chrono::Utc>,
}

#[test]
fn required_native_types_compile_without_adapters() {
    for filter in CASES {
        RequiredModel::compile_filter(filter).unwrap_or_else(|e| panic!("{filter}: {e}"));
        let field = filter.split('=').next().unwrap();
        RequiredModel::compile_filter(&format!("{field}:*")).unwrap();
        // Required PostgreSQL JSONB retains its JSON-null literal behavior.
        if !["jsonb_value", "text_value", "citext_value"].contains(&field) {
            assert!(
                RequiredModel::compile_filter(&format!("{field}=null")).is_err(),
                "{field}"
            );
        }
    }
}

//! Native type conformance: compile separately for each backend.
#![cfg(all(feature = "mysql", feature = "native-types"))]
#[cfg(feature = "mysql-integration")]
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
    tiny_value -> Nullable<TinyInt>,
    datetime_value -> Nullable<Datetime>,
    u8_value -> Nullable<Unsigned<TinyInt>>,
    u16_value -> Nullable<Unsigned<SmallInt>>,
    u32_value -> Nullable<Unsigned<Integer>>,
    u64_value -> Nullable<Unsigned<BigInt>>,
    ufloat_value -> Nullable<Unsigned<Float>>,
    udouble_value -> Nullable<Unsigned<Double>>,
    unumeric_value -> Nullable<Unsigned<Numeric>>,
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
    tiny_value: Option<i8>,
    datetime_value: Option<chrono::NaiveDateTime>,
    u8_value: Option<u8>,
    u16_value: Option<u16>,
    u32_value: Option<u32>,
    u64_value: Option<u64>,
    ufloat_value: Option<f32>,
    udouble_value: Option<f64>,
    unumeric_value: Option<bigdecimal::BigDecimal>,
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
    r#"tiny_value=127"#,
    r#"datetime_value="2026-10-17T12:34:56.123456""#,
    r#"u8_value=255"#,
    r#"u16_value=65535"#,
    r#"u32_value=4294967295"#,
    r#"u64_value=18446744073709551615"#,
    r#"ufloat_value=0.1"#,
    r#"udouble_value=2.5"#,
    r#"unumeric_value=12345678901234567890.123456789"#,
];

#[test]
fn native_types_compile_and_null_checks_bind_no_values() {
    for filter in CASES {
        let predicate = NativeModel::compile_filter(filter)
            .unwrap_or_else(|e| panic!("{filter}: {e}"))
            .unwrap();
        let sql = diesel::debug_query::<diesel::mysql::Mysql, _>(
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
            let sql = diesel::debug_query::<diesel::mysql::Mysql, _>(
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

#[cfg(feature = "mysql-integration")]
#[test]
#[ignore = "requires Compose database; run scripts/test-native-types.sh"]
fn native_types_execute_against_database() {
    let mut conn = diesel::mysql::MysqlConnection::establish(
        &std::env::var("AIP160_MYSQL_DATABASE_URL").unwrap(),
    )
    .unwrap();
    conn.batch_execute("CREATE TEMPORARY TABLE native_types (id INTEGER PRIMARY KEY, text_value Text, bool_value BOOLEAN, small_value SmallInt, integer_value Integer, big_value BigInt, float_value FLOAT, double_value DOUBLE, numeric_value DECIMAL(65,20), binary_value BLOB, date_value Date, time_value TIME(6), timestamp_value DATETIME(6), json_value Json, tiny_value TinyInt, datetime_value DATETIME(6), u8_value TINYINT UNSIGNED, u16_value SMALLINT UNSIGNED, u32_value INTEGER UNSIGNED, u64_value BIGINT UNSIGNED, ufloat_value FLOAT, udouble_value DOUBLE, unumeric_value DECIMAL(65,20)); INSERT INTO native_types VALUES (1,'Hello',true,32767,2147483647,9007199254740993,0.1,2.5,12345678901234567890.123456789,X'deadbeef','2026-10-17','12:34:56.123456','2026-10-17 12:34:56.123456','{\"a\":1}',127,'2026-10-17 12:34:56.123456',255,65535,4294967295,18446744073709551615,0.1,2.5,12345678901234567890.123456789); INSERT INTO native_types (id) VALUES (2);").unwrap();
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
    for filter in [
        "text_value:ELL",
        "text_value='He*'",
        "NOT bool_value=false",
        "u64_value>18446744073709551614",
        "numeric_value>12345678901234567890.123456788",
        "numeric_value<12345678901234567890.123456790",
    ] {
        let p = NativeModel::compile_filter(filter).unwrap().unwrap();
        assert_eq!(
            native_types::table
                .filter(p)
                .select(native_types::id)
                .load::<i32>(&mut conn)
                .unwrap(),
            vec![1],
            "{filter}"
        );
    }
    let p = NativeModel::compile_filter(r#"json_value:'{"a":1}'"#)
        .unwrap()
        .unwrap();
    assert_eq!(
        native_types::table
            .filter(p)
            .select(native_types::id)
            .load::<i32>(&mut conn)
            .unwrap(),
        vec![1]
    );
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
    tiny_value -> TinyInt,
    datetime_value -> Datetime,
    u8_value -> Unsigned<TinyInt>,
    u16_value -> Unsigned<SmallInt>,
    u32_value -> Unsigned<Integer>,
    u64_value -> Unsigned<BigInt>,
    ufloat_value -> Unsigned<Float>,
    udouble_value -> Unsigned<Double>,
    unumeric_value -> Unsigned<Numeric>,
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
    tiny_value: i8,
    datetime_value: chrono::NaiveDateTime,
    u8_value: u8,
    u16_value: u16,
    u32_value: u32,
    u64_value: u64,
    ufloat_value: f32,
    udouble_value: f64,
    unumeric_value: bigdecimal::BigDecimal,
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

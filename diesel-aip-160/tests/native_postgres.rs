//! Native type conformance: compile separately for each backend.
#![cfg(all(feature = "postgres", feature = "native-types"))]
#[cfg(feature = "postgres-integration")]
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
    interval_value -> Nullable<Interval>,
    json_value -> Nullable<Json>,
    jsonb_value -> Nullable<Jsonb>,
    uuid_value -> Nullable<Uuid>,
    tz_value -> Nullable<Timestamptz>,
    oid_value -> Nullable<Oid>,
    money_value -> Nullable<Money>,
    inet_value -> Nullable<Inet>,
    cidr_value -> Nullable<Cidr>,
    mac_value -> Nullable<MacAddr>,
    mac8_value -> Nullable<MacAddr8>,
    char_value -> Nullable<CChar>,
    citext_value -> Nullable<Citext>,
    lsn_value -> Nullable<PgLsn>,
    array_value -> Nullable<Array<Integer>>,
    range_value -> Nullable<Range<Integer>>,
    multi_value -> Nullable<Multirange<Integer>>,
    record_value -> Nullable<Record<(Integer, Text)>>,
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
    interval_value: Option<diesel::pg::data_types::PgInterval>,
    json_value: Option<serde_json::Value>,
    jsonb_value: Option<serde_json::Value>,
    uuid_value: Option<uuid::Uuid>,
    tz_value: Option<chrono::DateTime<chrono::Utc>>,
    oid_value: Option<u32>,
    money_value: Option<diesel::pg::data_types::PgMoney>,
    inet_value: Option<ipnet::IpNet>,
    cidr_value: Option<ipnet::IpNet>,
    mac_value: Option<[u8; 6]>,
    mac8_value: Option<[u8; 8]>,
    char_value: Option<u8>,
    citext_value: Option<String>,
    lsn_value: Option<diesel::pg::data_types::PgLsn>,
    array_value: Option<Vec<i32>>,
    range_value: Option<(std::ops::Bound<i32>, std::ops::Bound<i32>)>,
    multi_value: Option<Vec<(std::ops::Bound<i32>, std::ops::Bound<i32>)>>,
    record_value: Option<(i32, String)>,
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
    r#"interval_value="1 day 2 hours""#,
    r#"json_value='{"a":1}'"#,
    r#"jsonb_value='{"a":1}'"#,
    r#"uuid_value="123e4567-e89b-12d3-a456-426614174000""#,
    r#"tz_value="2026-10-17T12:34:56Z""#,
    r#"oid_value=4294967295"#,
    r#"money_value="12.34""#,
    r#"inet_value="192.168.1.7/24""#,
    r#"cidr_value="192.168.1.0/24""#,
    r#"mac_value="08:00:2b:01:02:03""#,
    r#"mac8_value="08:00:2b:01:02:03:04:05""#,
    r#"char_value="a""#,
    r#"citext_value="Hello""#,
    r#"lsn_value="16/B374D848""#,
    r#"array_value="{1,2,3}""#,
    r#"range_value="[1,10)""#,
    r#"multi_value="{[1,10),[20,30)}""#,
    r#"record_value='(7,"seven")'"#,
];

#[test]
fn native_types_compile_and_null_checks_bind_no_values() {
    for filter in CASES {
        let predicate = NativeModel::compile_filter(filter)
            .unwrap_or_else(|e| panic!("{filter}: {e}"))
            .unwrap();
        let sql = diesel::debug_query::<diesel::pg::Pg, _>(
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
            let sql = diesel::debug_query::<diesel::pg::Pg, _>(
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

#[cfg(feature = "postgres-integration")]
#[test]
#[ignore = "requires Compose database; run scripts/test-native-types.sh"]
fn native_types_execute_against_database() {
    let mut conn =
        diesel::pg::PgConnection::establish(&std::env::var("AIP160_TEST_DATABASE_URL").unwrap())
            .unwrap();
    conn.batch_execute("CREATE EXTENSION IF NOT EXISTS citext; DROP TYPE IF EXISTS pg_temp.aip160_pair CASCADE; CREATE TYPE pg_temp.aip160_pair AS (a integer,b text);").unwrap();
    conn.batch_execute("CREATE TEMPORARY TABLE native_types (id INTEGER PRIMARY KEY, text_value Text, bool_value Bool, small_value SmallInt, integer_value Integer, big_value BigInt, float_value real, double_value double precision, numeric_value Numeric, binary_value bytea, date_value Date, time_value Time, timestamp_value Timestamp, interval_value Interval, json_value Json, jsonb_value Jsonb, uuid_value Uuid, tz_value Timestamptz, oid_value Oid, money_value Money, inet_value Inet, cidr_value Cidr, mac_value MacAddr, mac8_value macaddr8, char_value \"char\", citext_value Citext, lsn_value pg_lsn, array_value integer[], range_value int4range, multi_value int4multirange, record_value pg_temp.aip160_pair); INSERT INTO native_types VALUES (1,'Hello',true,32767,2147483647,9007199254740993,0.1,2.5,12345678901234567890.123456789,decode('deadbeef','hex'),'2026-10-17','12:34:56.123456','2026-10-17T12:34:56.123456','1 day 2 hours','{\"a\":1}','{\"a\":1}','123e4567-e89b-12d3-a456-426614174000','2026-10-17T12:34:56Z',4294967295,'12.34','192.168.1.7/24','192.168.1.0/24','08:00:2b:01:02:03','08:00:2b:01:02:03:04:05','a','Hello','16/B374D848','{1,2,3}','[1,10)','{[1,10),[20,30)}',ROW(7,'seven')::pg_temp.aip160_pair); INSERT INTO native_types (id) VALUES (2);").unwrap();
    assert_range_families(&mut conn);
    // A structured nullable JSONB value uses the same SQL-type dispatch while
    // preserving serde key mappings.
    let p = MappedModel::compile_filter("jsonb_value.answer=1")
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
    for filter in [
        r#"record_value='(7,)'"#,
        r#"record_value="(7,\"x'); DROP TABLE native_types; --\")""#,
    ] {
        let p = NativeModel::compile_filter(filter).unwrap().unwrap();
        assert!(
            native_types::table
                .filter(p)
                .select(native_types::id)
                .load::<i32>(&mut conn)
                .unwrap()
                .is_empty(),
            "{filter}"
        );
    }
    assert_eq!(
        native_types::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        2
    );

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
        r#"array_value:"{2}""#,
        r#"range_value:"[2,4)""#,
        r#"multi_value:"{[2,4)}""#,
        r#"json_value:'{"a":1}'"#,
        r#"jsonb_value.a=1"#,
        "citext_value=hello",
        "citext_value:ELL",
        "NOT bool_value=false",
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
    diesel::update(native_types::table.filter(native_types::id.eq(2)))
        .set(native_types::char_value.eq(255u8))
        .execute(&mut conn)
        .unwrap();
    let p = NativeModel::compile_filter("char_value=255")
        .unwrap()
        .unwrap();
    assert_eq!(
        native_types::table
            .filter(p)
            .select(native_types::id)
            .load::<i32>(&mut conn)
            .unwrap(),
        vec![2]
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
    interval_value -> Interval,
    json_value -> Json,
    jsonb_value -> Jsonb,
    uuid_value -> Uuid,
    tz_value -> Timestamptz,
    oid_value -> Oid,
    money_value -> Money,
    inet_value -> Inet,
    cidr_value -> Cidr,
    mac_value -> MacAddr,
    mac8_value -> MacAddr8,
    char_value -> CChar,
    citext_value -> Citext,
    lsn_value -> PgLsn,
    array_value -> Array<Integer>,
    range_value -> Range<Integer>,
    multi_value -> Multirange<Integer>,
    record_value -> Record<(Integer, Text)>,
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
    interval_value: diesel::pg::data_types::PgInterval,
    json_value: serde_json::Value,
    jsonb_value: serde_json::Value,
    uuid_value: uuid::Uuid,
    tz_value: chrono::DateTime<chrono::Utc>,
    oid_value: u32,
    money_value: diesel::pg::data_types::PgMoney,
    inet_value: ipnet::IpNet,
    cidr_value: ipnet::IpNet,
    mac_value: [u8; 6],
    mac8_value: [u8; 8],
    char_value: u8,
    citext_value: String,
    lsn_value: diesel::pg::data_types::PgLsn,
    array_value: Vec<i32>,
    range_value: (std::ops::Bound<i32>, std::ops::Bound<i32>),
    multi_value: Vec<(std::ops::Bound<i32>, std::ops::Bound<i32>)>,
    record_value: (i32, String),
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

diesel::table! { range_types (id) { id -> Integer,
    int4_range -> Range<Integer>,
    int4_multi -> Multirange<Integer>,
    int8_range -> Range<BigInt>,
    int8_multi -> Multirange<BigInt>,
    num_range -> Range<Numeric>,
    num_multi -> Multirange<Numeric>,
    date_range -> Range<Date>,
    date_multi -> Multirange<Date>,
    ts_range -> Range<Timestamp>,
    ts_multi -> Multirange<Timestamp>,
    tstz_range -> Range<Timestamptz>,
    tstz_multi -> Multirange<Timestamptz>,
    array_nulls -> Array<Nullable<Integer>>,
} }
#[derive(Aip160Filter)]
#[diesel(table_name = range_types)]
#[allow(dead_code)]
struct RangeModel {
    id: i32,
    int4_range: (std::ops::Bound<i32>, std::ops::Bound<i32>),
    int4_multi: Vec<(std::ops::Bound<i32>, std::ops::Bound<i32>)>,
    int8_range: (std::ops::Bound<i64>, std::ops::Bound<i64>),
    int8_multi: Vec<(std::ops::Bound<i64>, std::ops::Bound<i64>)>,
    num_range: (
        std::ops::Bound<bigdecimal::BigDecimal>,
        std::ops::Bound<bigdecimal::BigDecimal>,
    ),
    num_multi: Vec<(
        std::ops::Bound<bigdecimal::BigDecimal>,
        std::ops::Bound<bigdecimal::BigDecimal>,
    )>,
    date_range: (
        std::ops::Bound<chrono::NaiveDate>,
        std::ops::Bound<chrono::NaiveDate>,
    ),
    date_multi: Vec<(
        std::ops::Bound<chrono::NaiveDate>,
        std::ops::Bound<chrono::NaiveDate>,
    )>,
    ts_range: (
        std::ops::Bound<chrono::NaiveDateTime>,
        std::ops::Bound<chrono::NaiveDateTime>,
    ),
    ts_multi: Vec<(
        std::ops::Bound<chrono::NaiveDateTime>,
        std::ops::Bound<chrono::NaiveDateTime>,
    )>,
    tstz_range: (
        std::ops::Bound<chrono::DateTime<chrono::Utc>>,
        std::ops::Bound<chrono::DateTime<chrono::Utc>>,
    ),
    tstz_multi: Vec<Bounds<chrono::DateTime<chrono::Utc>>>,
    array_nulls: Vec<Option<i32>>,
}

const RANGE_CASES: &[&str] = &[
    r#"int4_range='[1,10)'"#,
    r#"int4_multi='{[1,10)}'"#,
    r#"int8_range='[9007199254740993,9007199254740995)'"#,
    r#"int8_multi='{[9007199254740993,9007199254740995)}'"#,
    r#"num_range='[1.1,2.2)'"#,
    r#"num_multi='{[1.1,2.2)}'"#,
    r#"date_range='[2026-10-17,2026-10-19)'"#,
    r#"date_multi='{[2026-10-17,2026-10-19)}'"#,
    r#"ts_range='[2026-10-17,2026-10-19)'"#,
    r#"ts_multi='{[2026-10-17,2026-10-19)}'"#,
    r#"tstz_range='[2026-10-17,2026-10-19)'"#,
    r#"tstz_multi='{[2026-10-17,2026-10-19)}'"#,
    r#"array_nulls='{1,NULL,3}'"#,
];
#[test]
fn every_range_subtype_and_nullable_array_element_compiles() {
    for filter in RANGE_CASES {
        RangeModel::compile_filter(filter).unwrap_or_else(|e| panic!("{filter}: {e}"));
    }
}
#[cfg(feature = "postgres-integration")]
fn assert_range_families(conn: &mut diesel::pg::PgConnection) {
    conn.batch_execute("CREATE TEMPORARY TABLE range_types (id INTEGER PRIMARY KEY,int4_range int4range,int4_multi int4multirange,int8_range int8range,int8_multi int8multirange,num_range numrange,num_multi nummultirange,date_range daterange,date_multi datemultirange,ts_range tsrange,ts_multi tsmultirange,tstz_range tstzrange,tstz_multi tstzmultirange,array_nulls integer[]); INSERT INTO range_types VALUES (1,'[1,10)','{[1,10)}','[9007199254740993,9007199254740995)','{[9007199254740993,9007199254740995)}','[1.1,2.2)','{[1.1,2.2)}','[2026-10-17,2026-10-19)','{[2026-10-17,2026-10-19)}','[2026-10-17,2026-10-19)','{[2026-10-17,2026-10-19)}','[2026-10-17,2026-10-19)','{[2026-10-17,2026-10-19)}','{1,NULL,3}');").unwrap();
    for filter in RANGE_CASES {
        let p = RangeModel::compile_filter(filter).unwrap().unwrap();
        assert_eq!(
            range_types::table
                .filter(p)
                .select(range_types::id)
                .load::<i32>(conn)
                .unwrap(),
            vec![1],
            "{filter}"
        );
    }
}

#[derive(diesel_aip_160::Aip160Jsonb)]
#[allow(dead_code)]
struct MappedPayload {
    #[serde(rename = "a")]
    answer: i32,
}
#[derive(Aip160Filter)]
#[diesel(table_name = native_types)]
#[allow(dead_code)]
struct MappedModel {
    jsonb_value: Option<MappedPayload>,
}

type Bounds<T> = (std::ops::Bound<T>, std::ops::Bound<T>);

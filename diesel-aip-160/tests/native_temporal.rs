//! Compare against values serialized by Diesel, including nanoseconds and UTC.
#![cfg(all(feature = "sqlite", any(feature = "chrono", feature = "time")))]
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel_aip_160::Aip160Filter;

diesel::table! {
    temporal_values (id) {
        id -> Integer,
        day -> Date,
        clock -> Time,
        stamp -> Timestamp,
        instant -> TimestamptzSqlite,
    }
}
fn connection() -> diesel::sqlite::SqliteConnection {
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("CREATE TABLE temporal_values (id INTEGER PRIMARY KEY, day TEXT, clock TEXT, stamp TEXT, instant TEXT)").unwrap();
    conn
}
const FILTERS: &[(&str, bool)] = &[
    (r#"day="2026-10-17""#, true),
    (r#"clock="12:34:56.000000001""#, true),
    (r#"stamp="2026-10-17T12:34:56.000000001""#, true),
    (r#"instant="2026-10-17T20:34:56.000000001+08:00""#, true),
    (r#"instant="2026-10-17T12:34:56.000000001Z""#, true),
    (r#"instant="2026-10-17T12:34:56.000000002Z""#, false),
];
#[cfg(feature = "chrono")]
#[test]
fn chrono_values_match_diesel_storage_without_losing_nanoseconds() {
    #[derive(Aip160Filter)]
    #[diesel(table_name = temporal_values)]
    #[allow(dead_code)]
    struct Model {
        day: chrono::NaiveDate,
        clock: chrono::NaiveTime,
        stamp: chrono::NaiveDateTime,
        instant: chrono::DateTime<chrono::Utc>,
    }
    let mut conn = connection();
    diesel::insert_into(temporal_values::table)
        .values((
            temporal_values::id.eq(1),
            temporal_values::day.eq(chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()),
            temporal_values::clock.eq(chrono::NaiveTime::from_hms_nano_opt(12, 34, 56, 1).unwrap()),
            temporal_values::stamp.eq(chrono::NaiveDateTime::parse_from_str(
                "2026-10-17T12:34:56.000000001",
                "%Y-%m-%dT%H:%M:%S%.f",
            )
            .unwrap()),
            temporal_values::instant
                .eq(
                    chrono::DateTime::parse_from_rfc3339("2026-10-17T12:34:56.000000001Z").unwrap(),
                ),
        ))
        .execute(&mut conn)
        .unwrap();
    for (filter, expected) in FILTERS {
        let p = Model::compile_filter(filter).unwrap().unwrap();
        let sql =
            diesel::debug_query::<diesel::sqlite::Sqlite, _>(&temporal_values::table.filter(&p))
                .to_string();
        let stored = temporal_values::table
            .select(diesel::dsl::sql::<diesel::sql_types::Text>("clock"))
            .first::<String>(&mut conn)
            .unwrap();
        let count = temporal_values::table
            .filter(p)
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap();
        assert_eq!(
            count == 1,
            *expected,
            "{filter}: {sql}; stored clock {stored}"
        );
    }
}
#[cfg(feature = "time")]
#[test]
fn time_values_match_diesel_storage_and_timestamp_nanoseconds() {
    #[derive(Aip160Filter)]
    #[diesel(table_name = temporal_values)]
    #[allow(dead_code)]
    struct Model {
        day: time::Date,
        clock: time::Time,
        stamp: time::PrimitiveDateTime,
        instant: time::OffsetDateTime,
    }
    use time::format_description::well_known::Rfc3339;
    let mut conn = connection();
    let day = time::Date::from_calendar_date(2026, time::Month::October, 17).unwrap();
    let clock = time::Time::from_hms_nano(12, 34, 56, 1).unwrap();
    diesel::insert_into(temporal_values::table)
        .values((
            temporal_values::id.eq(1),
            temporal_values::day.eq(day),
            temporal_values::clock.eq(clock),
            temporal_values::stamp.eq(day.with_time(clock)),
            temporal_values::instant.eq(time::OffsetDateTime::parse(
                "2026-10-17T12:34:56.000000001Z",
                &Rfc3339,
            )
            .unwrap()),
        ))
        .execute(&mut conn)
        .unwrap();
    for (filter, expected) in FILTERS {
        let p = Model::compile_filter(filter).unwrap().unwrap();
        let sql =
            diesel::debug_query::<diesel::sqlite::Sqlite, _>(&temporal_values::table.filter(&p))
                .to_string();
        let stored = temporal_values::table
            .select(diesel::dsl::sql::<diesel::sql_types::Text>("clock"))
            .first::<String>(&mut conn)
            .unwrap();
        let count = temporal_values::table
            .filter(p)
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap();
        assert_eq!(
            count == 1,
            *expected,
            "{filter}: {sql}; stored clock {stored}"
        );
    }
}

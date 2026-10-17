//! Run separately with `--features sqlite`, `mysql`, or `postgres`.
#![cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]

use diesel::prelude::*;
use diesel_aip_160::Aip160Filter;

#[cfg(feature = "sqlite")]
type Backend = diesel::sqlite::Sqlite;
#[cfg(feature = "mysql")]
type Backend = diesel::mysql::Mysql;
#[cfg(feature = "postgres")]
type Backend = diesel::pg::Pg;

diesel::table! {
    records (id) {
        id -> Integer,
        name -> Text,
        note -> Nullable<Text>,
        small -> Nullable<Integer>,
        wide -> BigInt,
        order -> Nullable<BigInt>,
    }
}

#[derive(Aip160Filter)]
#[diesel(table_name = records)]
#[allow(dead_code)]
struct Record {
    id: i32,
    name: String,
    note: Option<String>,
    small: Option<i32>,
    wide: i64,
    order: Option<i64>,
}

fn sql(filter: &str) -> String {
    let predicate = Record::compile_filter(filter).unwrap().unwrap();
    diesel::debug_query::<Backend, _>(&records::table.filter(predicate)).to_string()
}

#[test]
fn feature_compiles_typed_predicates_with_backend_bind_syntax() {
    let query = sql("name=Alice AND id>=3 AND note:*");
    #[cfg(feature = "postgres")]
    {
        assert!(query.contains("\"records\".\"name\" = $1"), "{query}");
        assert!(query.contains("\"records\".\"id\" >= $2"), "{query}");
        assert!(
            query.contains("\"records\".\"note\" IS NOT NULL"),
            "{query}"
        );
    }
    #[cfg(any(feature = "sqlite", feature = "mysql"))]
    {
        assert!(query.contains("`records`.`name` = ?"), "{query}");
        assert!(query.contains("`records`.`id` >= ?"), "{query}");
        assert!(query.contains("`records`.`note` IS NOT NULL"), "{query}");
    }
    assert!(query.ends_with("-- binds: [\"Alice\", 3]"), "{query}");
}

#[test]
fn feature_handles_nullable_values_and_wildcards() {
    let null_query = sql("note=null");
    assert!(null_query.contains("IS NULL"), "{null_query}");
    assert!(null_query.ends_with("-- binds: []"), "{null_query}");

    let wildcard_query = sql("name='Al*'");
    assert!(wildcard_query.contains(" LIKE "), "{wildcard_query}");
    assert!(
        wildcard_query.contains("-- binds: [\"Al%\""),
        "{wildcard_query}"
    );
}

#[test]
fn feature_rejects_unknown_fields_and_empty_filter_has_no_predicate() {
    assert!(Record::compile_filter("unknown=Alice").is_err());
    assert!(Record::compile_filter(" ").unwrap().is_none());
}

#[test]
fn substring_and_global_search_lower_both_operands_only() {
    for filter in ["name:ALi", "note:ALi", "ALi"] {
        let query = sql(filter);
        #[cfg(feature = "postgres")]
        assert!(query.contains("lower(\"records\"."), "{query}");
        #[cfg(any(feature = "sqlite", feature = "mysql"))]
        assert!(query.contains("lower(`records`."), "{query}");
        assert!(query.contains(") LIKE lower("), "{query}");
        assert!(query.contains("%ALi%"), "{query}");
        assert!(query.contains("ESCAPE"), "{query}");
    }
    for filter in [
        "name=ALi",
        "name!=ALi",
        "name='ALi*'",
        "name!='ALi*'",
        "note:*",
    ] {
        let query = sql(filter);
        assert!(!query.contains("lower("), "{query}");
    }
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_executes_case_insensitive_search_with_literal_patterns_and_nulls() {
    let mut connection = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    diesel::sql_query(
        "CREATE TABLE records (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, small INTEGER, wide BIGINT NOT NULL DEFAULT 0, \"order\" BIGINT)",
    )
    .execute(&mut connection)
    .unwrap();
    for (id, name, note) in [
        (1, "Alice", None),
        (2, "MALIK", Some("alley")),
        (3, "Bob", Some("ALICE")),
        (4, r"Al*_%\X", None),
        (5, "AlZZ", Some("other")),
    ] {
        diesel::insert_into(records::table)
            .values((
                records::id.eq(id),
                records::name.eq(name),
                records::note.eq(note),
            ))
            .execute(&mut connection)
            .unwrap();
    }
    for (filter, expected) in [
        ("name:ali", vec![1, 2]),
        ("name:ALI", vec![1, 2]),
        ("note:ali", vec![3]),
        ("ali", vec![1, 2, 3]),
        ("NOT note:ali", vec![2, 5]),
        ("name:(ali OR bob)", vec![1, 2, 3]),
        (r"name:'AL*_%\\x'", vec![4]),
        ("name:'*'", vec![4]),
        ("name=alice", vec![]),
        ("name=Alice", vec![1]),
        ("name='Al*'", vec![1, 4, 5]),
        ("note:*", vec![2, 3, 5]),
    ] {
        let predicate = Record::compile_filter(filter).unwrap().unwrap();
        let ids = records::table
            .filter(predicate)
            .select(records::id)
            .order(records::id)
            .load::<i32>(&mut connection)
            .unwrap();
        assert_eq!(ids, expected, "{filter}");
    }
}

#[test]
fn integer_ranges_nulls_and_parameter_binding() {
    for field in ["id", "small"] {
        for value in [i32::MIN as i64, i32::MAX as i64] {
            assert!(sql(&format!("{field}={value}")).ends_with(&format!("-- binds: [{value}]")));
        }
        for value in ["2147483648", "-2147483649"] {
            let error = Record::compile_filter(&format!("{field}={value}"))
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("32-bit") && error.contains("overflow"),
                "{error}"
            );
        }
    }
    for field in ["wide", "order"] {
        for value in [i64::MIN, i64::MAX, 4294967295, 9007199254740993] {
            for operator in ["=", "!=", "<", "<=", ">", ">="] {
                let query = sql(&format!("{field}{operator}{value}"));
                assert!(query.ends_with(&format!("-- binds: [{value}]")), "{query}");
            }
        }
        for value in [
            "9223372036854775808",
            "-9223372036854775809",
            "'5'",
            "1.5",
            "1e3",
        ] {
            assert!(Record::compile_filter(&format!("{field}={value}")).is_err());
        }
    }
    for field in ["small", "order"] {
        assert!(sql(&format!("{field}=null")).contains("IS NULL"));
        assert!(sql(&format!("{field}!=null")).contains("IS NOT NULL"));
        assert!(sql(&format!("{field}:*")).contains("IS NOT NULL"));
        for operator in ["<", "<=", ">", ">="] {
            assert!(Record::compile_filter(&format!("{field}{operator}null")).is_err());
        }
    }
    assert!(Record::compile_filter("wide=null").is_err());
    let query = sql("(small=null OR small>=-2147483648) AND NOT order<4294967295");
    assert!(query.contains(" OR ") && query.contains(" AND ") && query.contains("NOT"));
    assert!(
        query.ends_with("-- binds: [-2147483648, 4294967295]"),
        "{query}"
    );
    assert!(!query.contains("COALESCE"));
}

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
    }
}

#[derive(Aip160Filter)]
#[diesel(table_name = records)]
#[allow(dead_code)]
struct Record {
    id: i32,
    name: String,
    note: Option<String>,
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
        assert!(query.contains("\"records\".\"note\" IS NOT NULL"), "{query}");
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
    assert!(wildcard_query.contains("-- binds: [\"Al%\""), "{wildcard_query}");
}

#[test]
fn feature_rejects_unknown_fields_and_empty_filter_has_no_predicate() {
    assert!(Record::compile_filter("unknown=Alice").is_err());
    assert!(Record::compile_filter(" ").unwrap().is_none());
}

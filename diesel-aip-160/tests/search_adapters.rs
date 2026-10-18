//! Search adapters compile independently on each backend.
#![cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use diesel::prelude::*;
use diesel_aip_160::{
    Aip160Filter,
    diesel_helpers::{Predicate, SearchAdapter},
    parser::Value,
};
#[cfg(feature = "postgres")]
type Backend = diesel::pg::Pg;
#[cfg(feature = "mysql")]
type Backend = diesel::mysql::Mysql;
#[cfg(feature = "sqlite")]
type Backend = diesel::sqlite::Sqlite;

diesel::table! { documents (id) { id -> Integer, title -> Nullable<Text>, body -> Nullable<Text> } }

struct TitleSearch;
impl SearchAdapter<documents::table> for TitleSearch {
    fn search(value: &Value) -> diesel_aip_160::anyhow::Result<Predicate<documents::table>> {
        // The transformed bind proves adapters receive quoting and raw text.
        let query = format!("{}:{}", value.quoted, value.text);
        Ok(Box::new(documents::body.eq(query).assume_not_null()))
    }
}
struct GlobalSearch;
impl SearchAdapter<documents::table> for GlobalSearch {
    fn search(value: &Value) -> diesel_aip_160::anyhow::Result<Predicate<documents::table>> {
        Ok(Box::new(
            documents::body.eq(value.text.clone()).assume_not_null(),
        ))
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = documents)]
#[allow(dead_code)]
struct Document {
    id: i32,
    #[aip160(search_adapter = TitleSearch)]
    title: Option<String>,
}
#[derive(Aip160Filter)]
#[diesel(table_name = documents)]
#[aip160(global_search_adapter = GlobalSearch)]
#[allow(dead_code)]
struct GlobalDocument {
    id: i32,
}
fn sql(source: &str, global: bool) -> String {
    let predicate = if global {
        GlobalDocument::compile_filter(source)
    } else {
        Document::compile_filter(source)
    }
    .unwrap()
    .unwrap();
    diesel::debug_query::<Backend, _>(&documents::table.filter(predicate).select(documents::id))
        .to_string()
}
#[test]
fn custom_field_search_and_global_expansion_receive_raw_values() {
    for (source, bind) in [
        ("title:001", "false:001"),
        ("title:'001'", "true:001"),
        ("title:'a%_*'", "true:a%_*"),
        ("'diesel engine'", "true:diesel engine"),
    ] {
        let query = sql(source, false);
        assert!(query.contains("body"), "{query}");
        assert!(query.ends_with(&format!("-- binds: [{bind:?}]")), "{query}");
        assert!(!query.contains(" LIKE "));
    }
    for source in [
        "title:*",
        "title=null",
        "title='a*'",
        "title=001",
        "title!=001",
        "title>001",
    ] {
        let query = sql(source, false);
        assert!(!query.contains("body"), "{source}: {query}");
    }
    assert!(Document::compile_filter("title:null").is_err());
    assert!(sql("NOT title:(a OR b) AND id>=1", false).contains("NOT"));
    assert!(sql("NOT title:(a OR b) AND id>=1", false).contains(" OR "));
}
#[test]
fn global_adapter_works_without_searchable_fields() {
    let query = sql("'diesel engine'", true);
    assert!(query.contains("body"));
    assert!(query.ends_with("-- binds: [\"diesel engine\"]"));
    assert!(!query.contains(" OR "));
    let query = sql("id=1 engine", true);
    assert!(query.contains(" AND "));
    assert!(query.ends_with("-- binds: [1, \"engine\"]"));
}

#[cfg(feature = "sqlite")]
#[test]
fn custom_search_executes_and_keeps_null_semantics_under_not() {
    use diesel::connection::SimpleConnection;
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("CREATE TABLE documents (id integer primary key, title text, body text);
        INSERT INTO documents VALUES (1,'unused','false:diesel'), (2,'unused','other'), (3,NULL,NULL);").unwrap();
    for (source, expected) in [
        ("title:diesel", vec![1]),
        ("NOT title:diesel", vec![2]),
        ("title=null", vec![3]),
    ] {
        let predicate = Document::compile_filter(source).unwrap().unwrap();
        assert_eq!(
            documents::table
                .filter(predicate)
                .select(documents::id)
                .order(documents::id)
                .load::<i32>(&mut conn)
                .unwrap(),
            expected
        );
    }
}

// Combining comparison and search adapters does not collapse ':' to equality.
struct Code;
struct CodeAdapter;
impl diesel_aip_160::diesel_helpers::FieldAdapter<documents::id> for CodeAdapter {
    fn compare(
        column: documents::id,
        operator: diesel_aip_160::compiler::ComparisonOperator,
        value: &Value,
    ) -> diesel_aip_160::anyhow::Result<Predicate<documents::table>> {
        use diesel_aip_160::compiler::ComparisonOperator;
        let code: i32 = value.text.parse()?;
        match operator {
            ComparisonOperator::Equal => Ok(Box::new(column.eq(code))),
            ComparisonOperator::NotEqual => Ok(Box::new(column.ne(code))),
            _ => diesel_aip_160::anyhow::bail!("unsupported code comparison"),
        }
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = documents)]
#[allow(dead_code)]
struct CustomDocument {
    #[aip160(adapter = CodeAdapter, search_adapter = TitleSearch)]
    id: Code,
}
#[test]
fn independent_search_and_comparison_adapters_can_coexist() {
    for (source, bind, target) in [
        ("id:001", "[\"false:001\"]", "body"),
        ("001", "[\"false:001\"]", "body"),
        ("id=001", "[1]", "id"),
        ("id!=001", "[1]", "id"),
        ("id:*", "[true]", ""),
    ] {
        let predicate = CustomDocument::compile_filter(source).unwrap().unwrap();
        let query = diesel::debug_query::<Backend, _>(
            &documents::table.filter(predicate).select(documents::id),
        )
        .to_string();
        assert!(
            query.ends_with(&format!("-- binds: {bind}")),
            "{source}: {query}"
        );
        if !target.is_empty() {
            assert!(query.contains(target), "{query}");
        }
    }
    assert!(CustomDocument::compile_filter("id:null").is_err());
}

#[test]
fn existing_adapters_need_no_rank_implementation() {
    for source in [
        "title:001",
        "'diesel engine'",
        "NOT title:(a OR b)",
        "title:*",
        "id=3",
    ] {
        let legacy = Document::compile_filter(source).unwrap().unwrap();
        let query = Document::compile_query_filter(source).unwrap().unwrap();
        assert!(query.signals.is_empty());
        assert_eq!(
            diesel::debug_query::<Backend, _>(&documents::table.filter(legacy)).to_string(),
            diesel::debug_query::<Backend, _>(&documents::table.filter(query.predicate))
                .to_string()
        );
    }
    assert!(
        GlobalDocument::compile_query_filter("diesel")
            .unwrap()
            .unwrap()
            .signals
            .is_empty()
    );
}

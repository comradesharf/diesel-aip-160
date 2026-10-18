//! Run with --features postgres. See README for application-owned database setup.
use diesel::prelude::*;
use diesel_aip_160::{
    Aip160Filter,
    diesel_helpers::{Predicate, SearchAdapter},
    parser::Value,
    search::postgres,
};

diesel::table! {
    use diesel::sql_types::*;
    use diesel_aip_160::search::postgres::TsVector;
    articles (id) {
        id -> Integer,
        title -> Nullable<Text>,
        body -> Nullable<Text>,
        document -> Nullable<TsVector>,
        published -> Bool,
    }
}

#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[aip160(global_search_adapter = CombinedDocument)]
#[allow(dead_code)]
struct Article {
    id: i32,
    #[aip160(search(postgres, configuration = "english"))]
    title: Option<String>,
    body: Option<String>, // Retains escaped, case-insensitive substring search.
    published: bool,
}

struct CombinedDocument;
impl SearchAdapter<articles::table> for CombinedDocument {
    fn search(value: &Value) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
        postgres::vector(articles::document, "english", value)
    }
}

fn main() -> diesel_aip_160::anyhow::Result<()> {
    let filter =
        r#"title:"diesel engine" AND body:manual AND published=true AND id>=10 AND maintenance"#;
    let predicate = Article::compile_filter(filter)?.unwrap();
    let query = articles::table.filter(predicate).select(articles::id);
    println!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    Ok(())
}

//! Run with --features postgres. See README for application-owned database setup.
use diesel::prelude::*;
use diesel_aip_160::{
    Aip160Filter,
    diesel_helpers::{Predicate, SearchAdapter},
    search::{SearchTerm, autocomplete_field, postgres},
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
    const SUPPORTS_PREFIX: bool = true;
    fn search_query(
        value: &SearchTerm,
    ) -> diesel_aip_160::anyhow::Result<diesel_aip_160::ranking::CompiledQueryFilter<articles::table>>
    {
        postgres::vector_query(articles::document, "english", value)
    }

    fn search(value: &SearchTerm) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
        postgres::vector(articles::document, "english", value)
    }
}

fn main() -> diesel_aip_160::anyhow::Result<()> {
    let filter =
        r#"title:"diesel engine" AND body:manual AND published=true AND id>=10 AND maintenance"#;
    let predicate = Article::compile_filter(filter)?.unwrap();
    let query = articles::table.filter(predicate).select(articles::id);
    println!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    let mut compiled = Article::compile_query_filter(filter)?.unwrap();
    let order = compiled.ordering(&["relevance"])?;
    let ranked = articles::table
        .filter(compiled.predicate)
        .select(articles::id)
        .order(order)
        .then_order_by(articles::id)
        .limit(20)
        .offset(40);
    println!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&ranked));
    // Arbitrary autocomplete text is never parsed as filter syntax.
    if let Some(expression) = autocomplete_field("title", "diesel eng")? {
        let mut compiled = Article::compile_search(&expression)?;
        let order = compiled.ordering(&["relevance"])?;
        let query = articles::table
            .filter(compiled.predicate)
            .order(order)
            .then_order_by(articles::id)
            .limit(20);
        println!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    }
    let _ = Article::compile_query_filter("title:eng* OR maint*")?;
    Ok(())
}

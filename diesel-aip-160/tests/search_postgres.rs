//! PostgreSQL search SQL tests; live tests run through the existing Compose suite.
#![cfg(feature = "postgres")]
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
        note -> Nullable<Text>,
        document -> Nullable<TsVector>,
        payload -> Jsonb,
    }
}

#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[allow(dead_code)]
struct Article {
    id: i32,
    #[aip160(search(postgres, configuration = "english"))]
    title: Option<String>,
    note: Option<String>,
    payload: serde_json::Value,
}

#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[aip160(global_search_adapter = DocumentSearch)]
#[allow(dead_code)]
struct DocumentArticle {
    id: i32,
    #[aip160(search(postgres, configuration = "english", vector = articles::document))]
    title: Option<String>,
    note: Option<String>,
}
struct DocumentSearch;
impl SearchAdapter<articles::table> for DocumentSearch {
    fn search(value: &Value) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
        postgres::vector(articles::document, "english", value)
    }
}
fn sql(source: &str, combined: bool) -> String {
    let p = if combined {
        DocumentArticle::compile_filter(source)
    } else {
        Article::compile_filter(source)
    }
    .unwrap()
    .unwrap();
    diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.filter(p).select(articles::id))
        .to_string()
}
#[test]
fn full_text_binds_query_and_uses_registered_target_and_configuration() {
    let query = sql("title:diesel AND id>=7", false);
    assert!(query.contains("to_tsvector('english'::regconfig, \"articles\".\"title\") @@ plainto_tsquery('english'::regconfig, $1)"), "{query}");
    assert!(query.ends_with("-- binds: [\"diesel\", 7]"), "{query}");
    let query = sql(r#"title:"diesel engine""#, false);
    assert!(
        query.contains("phraseto_tsquery('english'::regconfig, $1)"),
        "{query}"
    );
    assert!(query.ends_with("-- binds: [\"diesel engine\"]"), "{query}");
    let query = sql(r#"title:"x'); DROP TABLE articles; --""#, false);
    let (statement, binds) = query.split_once("-- binds:").unwrap();
    assert!(!statement.contains("DROP TABLE"));
    assert!(binds.contains("DROP TABLE"));
    let query = sql("title:diesel", true);
    assert!(
        query.contains("\"articles\".\"document\" @@ plainto_tsquery"),
        "{query}"
    );
    assert!(!query.contains("to_tsvector"));
}
#[test]
fn default_global_uses_field_search_and_explicit_global_replaces_expansion() {
    let default = sql("diesel", false);
    assert!(default.contains("to_tsvector"));
    assert!(default.contains(" OR "));
    assert!(default.contains("lower(\"articles\".\"note\")"));
    assert!(
        default.ends_with("-- binds: [\"diesel\", \"%diesel%\", \"\\\\\"]"),
        "{default}"
    );
    let global = sql("diesel", true);
    assert!(global.contains("\"document\" @@ plainto_tsquery"));
    assert!(!global.contains(" OR "));
    assert!(!global.contains("\"note\""));
    assert!(global.ends_with("-- binds: [\"diesel\"]"));
    let grammar = sql("title:diesel engine", false);
    assert!(grammar.contains(" AND "));
    assert!(!grammar.contains("phraseto_tsquery"));
    assert!(grammar.contains("OR"));
}
#[test]
fn search_opt_in_keeps_comparisons_presence_nulls_and_json_containment() {
    for source in [
        "title=Diesel",
        "title!=Diesel",
        "title>Diesel",
        "title='Die*'",
        "title!='Die*'",
        "title:*",
        "title=null",
        "title!=null",
        "NOT title:*",
        "note:die",
        "payload:'{\"kind\":\"diesel\"}'",
    ] {
        let query = sql(source, false);
        assert!(!query.contains("tsquery"), "{source}: {query}");
    }
    assert!(sql("note:die", false).contains(" LIKE "));
    assert!(sql("payload:'{\"kind\":\"diesel\"}'", false).contains(" @> "));
    assert!(Article::compile_filter("title:null").is_err());
    assert!(sql("title:'null'", false).contains("phraseto_tsquery"));
    let query = sql("NOT title:diesel", false);
    assert!(query.contains("NOT"));
    assert!(!query.to_lowercase().contains("coalesce"));
    assert!(
        postgres::text(
            articles::title,
            "english';--",
            &Value {
                text: "x".into(),
                quoted: false
            }
        )
        .is_err()
    );
}

#[cfg(feature = "postgres-integration")]
#[test]
#[ignore = "requires Compose database; run scripts/test-native-types.sh"]
fn full_text_executes_with_stemming_phrases_nulls_and_combined_documents() {
    use diesel::connection::SimpleConnection;
    let mut conn =
        diesel::pg::PgConnection::establish(&std::env::var("AIP160_TEST_DATABASE_URL").unwrap())
            .unwrap();
    conn.batch_execute("CREATE TEMP TABLE articles (id integer PRIMARY KEY, title text, note text, document tsvector, payload jsonb NOT NULL DEFAULT '{}');
        INSERT INTO articles(id,title,note) VALUES
        (1,'diesel engines','manual'), (2,'diesel powerful engine','notes'),
        (3,'biodiesel motor','engine'), (4,NULL,NULL), (5,'running quickly','manual');
        UPDATE articles SET document = to_tsvector('english', coalesce(title,'') || ' ' || coalesce(note,''));
        CREATE INDEX article_title_fts ON articles USING GIN (to_tsvector('english'::regconfig,title));
        CREATE INDEX article_document_fts ON articles USING GIN(document);").unwrap();
    let mut ids = |source: &str, combined: bool| {
        let p = if combined {
            DocumentArticle::compile_filter(source)
        } else {
            Article::compile_filter(source)
        }
        .unwrap()
        .unwrap();
        articles::table
            .filter(p)
            .select(articles::id)
            .order(articles::id)
            .load::<i32>(&mut conn)
            .unwrap()
    };
    for (source, expected) in [
        ("title:diesel", vec![1, 2]), // No substring match inside biodiesel.
        ("title:run", vec![5]),       // English stemming, unlike substring search.
        ("title:'diesel engine'", vec![1]),
        ("title:diesel engine", vec![1, 2]),
        ("NOT title:diesel", vec![3, 5]), // NULL remains unknown under NOT.
        ("title:*", vec![1, 2, 3, 5]),
        ("title=null", vec![4]),
        ("title='diesel engines'", vec![1]),
        ("title='diesel*'", vec![1, 2]),
        ("note:man", vec![1, 5]),
        ("title:'the'", vec![]),
        ("title:''", vec![]),
        ("NOT (title:diesel OR id=5)", vec![3]),
    ] {
        assert_eq!(ids(source, false), expected, "{source}");
    }
    assert_eq!(ids("manual", true), vec![1, 5]);
    assert_eq!(ids("title:manual", true), vec![1, 5]);
    assert_eq!(ids("NOT manual", true), vec![2, 3, 4]); // Document deliberately coalesces missing text.
}

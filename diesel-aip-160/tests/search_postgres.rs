//! PostgreSQL search SQL tests; live tests run through the existing Compose suite.
#![cfg(feature = "postgres")]
use diesel::prelude::*;
use diesel_aip_160::{
    Aip160Filter,
    diesel_helpers::{Predicate, SearchAdapter},
    search::SearchTerm,
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
    const SUPPORTS_PREFIX: bool = true;
    fn search(
        value: &::diesel_aip_160::search::SearchTerm,
    ) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
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
    assert!(postgres::text(articles::title, "english';--", &SearchTerm::exact("x")).is_err());
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
        ("title:run", vec![5]),
        ("title:r*", vec![5]),
        ("title:running*", vec![5]),
        ("title:the*", vec![]),
        ("title:'the&!!!'", vec![]), // English stemming, unlike substring search.
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
    assert_eq!(ids("man*", true), vec![1, 5]);
    assert_eq!(ids("manual", true), vec![1, 5]);
    assert_eq!(ids("title:manual", true), vec![1, 5]);
    assert_eq!(ids("NOT manual", true), vec![2, 3, 4]); // Document deliberately coalesces missing text.
}

#[test]
fn ranked_query_shares_predicate_and_binds_all_rank_values() {
    for source in [
        "title:diesel",
        "title:'diesel engine'",
        "diesel",
        "NOT title:diesel",
        "title:*",
        "id=7",
        "title='x*'",
        "title:null",
    ] {
        let legacy = Article::compile_filter(source);
        let ranked = Article::compile_query_filter(source);
        if legacy.is_err() {
            assert!(ranked.is_err());
            continue;
        }
        let legacy = legacy.unwrap().unwrap();
        let ranked = ranked.unwrap().unwrap();
        assert_eq!(
            diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.filter(legacy)).to_string(),
            diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.filter(ranked.predicate))
                .to_string(),
            "{source}"
        );
    }
    for (source, tsquery) in [
        ("title:diesel", "plainto_tsquery"),
        ("title:'diesel engine'", "phraseto_tsquery"),
    ] {
        let mut result = Article::compile_query_filter(source).unwrap().unwrap();
        assert_eq!(result.signals.len(), 1);
        assert_eq!(result.signals[0].operation, "postgres.full_text");
        let order = result.ordering(&["relevance"]).unwrap();
        let query = articles::table
            .filter(result.predicate)
            .order(order)
            .then_order_by(articles::id)
            .limit(10)
            .offset(2);
        let sql = diesel::debug_query::<diesel::pg::Pg, _>(&query).to_string();
        assert!(
            sql.contains(
                "COALESCE(ts_rank_cd(to_tsvector('english'::regconfig, \"articles\".\"title\"), "
            ),
            "{sql}"
        );
        assert_eq!(sql.matches(tsquery).count(), 2);
        assert!(sql.contains("0.0::real) DESC"));
        assert!(sql.find("ORDER BY").unwrap() < sql.find("LIMIT").unwrap());
    }
    let mut result = Article::compile_query_filter(r#"title:"x'); DROP TABLE articles; --""#)
        .unwrap()
        .unwrap();
    let rank = result
        .take_signal("relevance")
        .unwrap()
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let query = articles::table
        .filter(result.predicate)
        .select((articles::id, rank));
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(&query).to_string();
    let (statement, binds) = sql.split_once("-- binds:").unwrap();
    assert!(!statement.contains("DROP TABLE"));
    assert_eq!(binds.matches("DROP TABLE").count(), 2);
}

#[test]
fn text_signal_boolean_polarity_global_expansion_and_deduplication() {
    for source in [
        "NOT title:diesel",
        "NOT (title:diesel OR title:engine)",
        "id=1",
        "note:diesel",
    ] {
        assert!(
            Article::compile_query_filter(source)
                .unwrap()
                .unwrap()
                .signals
                .is_empty(),
            "{source}"
        );
    }
    for source in [
        "NOT (NOT title:diesel)",
        "title:(diesel OR NOT engine)",
        "NOT (NOT title:diesel AND title:engine)",
        "title:diesel OR id=1",
        "diesel",
        "title:diesel OR title:diesel",
        "diesel AND title:diesel",
    ] {
        let mut result = Article::compile_query_filter(source).unwrap().unwrap();
        assert_eq!(result.signals.len(), 1, "{source}");
        let rank = result
            .take_signal("relevance")
            .unwrap()
            .into_expression::<diesel::sql_types::Float>()
            .unwrap();
        let sql =
            diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.select(rank)).to_string();
        assert_eq!(sql.matches("ts_rank_cd").count(), 1, "{source}: {sql}");
        assert!(!sql.contains("engine"), "{source}: {sql}");
    }
    let mut result = Article::compile_query_filter("title:(diesel AND engine)")
        .unwrap()
        .unwrap();
    let rank = result
        .take_signal("relevance")
        .unwrap()
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.select(rank)).to_string();
    assert_eq!(sql.matches("ts_rank_cd").count(), 2);
    assert!(sql.contains("GREATEST("));
    // Existing global adapters emit no signals unless they opt into search_query.
    assert!(
        DocumentArticle::compile_query_filter("diesel")
            .unwrap()
            .unwrap()
            .signals
            .is_empty()
    );
    let mut stored = DocumentArticle::compile_query_filter("title:'diesel engine'")
        .unwrap()
        .unwrap();
    let order = stored.ordering(&["relevance"]).unwrap();
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(
        &articles::table.filter(stored.predicate).order(order),
    )
    .to_string();
    assert!(
        sql.contains("ts_rank_cd(\"articles\".\"document\", phraseto_tsquery('english'::regconfig")
    );
    assert!(!sql.contains("to_tsvector"));
}

#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[allow(dead_code)]
struct AllTextArticle {
    id: i32,
    #[aip160(search(postgres, configuration = "english"))]
    title: Option<String>,
    #[aip160(search(postgres, configuration = "simple"))]
    note: Option<String>,
}
struct RankedDocumentSearch;
impl SearchAdapter<articles::table> for RankedDocumentSearch {
    const SUPPORTS_PREFIX: bool = true;
    fn search(
        value: &::diesel_aip_160::search::SearchTerm,
    ) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
        postgres::vector(articles::document, "english", value)
    }
    fn search_query(
        value: &::diesel_aip_160::search::SearchTerm,
    ) -> diesel_aip_160::anyhow::Result<diesel_aip_160::ranking::CompiledQueryFilter<articles::table>>
    {
        postgres::vector_query(articles::document, "english", value)
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[aip160(global_search_adapter = RankedDocumentSearch)]
#[allow(dead_code)]
struct RankedDocumentArticle {
    #[aip160(search(postgres, configuration = "english", vector = articles::document))]
    title: Option<String>,
    #[aip160(search(postgres, configuration = "english", vector = articles::document))]
    note: Option<String>,
}

#[test]
fn global_text_uses_each_configuration_and_shared_vectors_are_not_counted_twice() {
    let mut result = AllTextArticle::compile_query_filter("engine")
        .unwrap()
        .unwrap();
    let rank = result
        .take_signal("relevance")
        .unwrap()
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let query = articles::table.filter(result.predicate).select(rank);
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(&query).to_string();
    assert_eq!(sql.matches("ts_rank_cd").count(), 2);
    for configuration in ["english", "simple"] {
        assert_eq!(
            sql.matches(&format!("plainto_tsquery('{configuration}'"))
                .count(),
            2,
            "{sql}"
        );
    }
    let mut result =
        RankedDocumentArticle::compile_query_filter("engine OR title:engine OR note:engine")
            .unwrap()
            .unwrap();
    let rank = result
        .take_signal("relevance")
        .unwrap()
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(&articles::table.select(rank)).to_string();
    assert_eq!(sql.matches("ts_rank_cd").count(), 1);
}

#[cfg(feature = "postgres-integration")]
#[test]
#[ignore = "requires Compose database; run scripts/test-native-types.sh"]
fn ranking_executes_with_nulls_boolean_policy_deduplication_and_pagination() {
    use diesel::connection::SimpleConnection;
    let mut conn =
        diesel::pg::PgConnection::establish(&std::env::var("AIP160_TEST_DATABASE_URL").unwrap())
            .unwrap();
    conn.batch_execute("CREATE TEMP TABLE articles (id integer PRIMARY KEY, title text, note text, document tsvector, payload jsonb NOT NULL DEFAULT '{}');
        INSERT INTO articles(id,title,note) VALUES
        (1,'diesel engine diesel engine',NULL), (2,'diesel powerful engine',NULL),
        (3,NULL,'engine manual'), (4,NULL,NULL);
        UPDATE articles SET document = to_tsvector('english', title);").unwrap();
    let mut ranked = |source: &str| {
        let mut result = Article::compile_query_filter(source).unwrap().unwrap();
        let rank = result
            .take_signal("relevance")
            .unwrap()
            .into_expression::<diesel::sql_types::Float>()
            .unwrap();
        articles::table
            .filter(result.predicate)
            .select((articles::id, rank))
            .order(articles::id)
            .load::<(i32, f32)>(&mut conn)
            .unwrap()
    };
    let scores = ranked("title:diesel OR id>=3");
    assert!(scores[0].1 > scores[1].1);
    assert_eq!(&scores[2..], &[(3, 0.0), (4, 0.0)]);
    assert_eq!(scores, ranked("title:diesel OR title:diesel OR id>=3"));
    assert_eq!(
        scores,
        ranked("title:diesel OR title:'diesel' OR title:DIESELS OR id>=3")
    );
    assert_eq!(scores, ranked("NOT (NOT title:diesel) OR id>=3"));
    // Signals are collected independently of branch truth, by documented policy.
    assert_eq!(scores, ranked("(title:diesel AND id=99) OR id>=1"));
    let negative = ranked("(title:diesel AND NOT title:manual) OR id>=3");
    assert_eq!(scores, negative);
    assert_eq!(
        ranked("title:'diesel engine'")
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(
        ranked("title:'the' OR id>=1")
            .iter()
            .all(|(_, score)| *score == 0.0)
    );
    let mut result = Article::compile_query_filter("title:diesel OR id>=3")
        .unwrap()
        .unwrap();
    let order = result.ordering(&["relevance"]).unwrap();
    let ids = articles::table
        .filter(result.predicate)
        .select(articles::id)
        .order(order)
        .then_order_by(articles::id)
        .limit(2)
        .offset(1)
        .load::<i32>(&mut conn)
        .unwrap();
    assert_eq!(ids, vec![2, 3]);
    let mut result = AllTextArticle::compile_query_filter("engine")
        .unwrap()
        .unwrap();
    let order = result.ordering(&["relevance"]).unwrap();
    let ids = articles::table
        .filter(result.predicate)
        .select(articles::id)
        .order(order)
        .then_order_by(articles::id)
        .load::<i32>(&mut conn)
        .unwrap();
    assert_eq!(ids, vec![1, 2, 3]);
    let mut result = RankedDocumentArticle::compile_query_filter("'diesel engine'")
        .unwrap()
        .unwrap();
    let order = result.ordering(&["relevance"]).unwrap();
    assert_eq!(
        articles::table
            .filter(result.predicate)
            .select(articles::id)
            .order(order)
            .load::<i32>(&mut conn)
            .unwrap(),
        vec![1]
    );
    // Selecting Float proves that COALESCE returns a nonnullable real value.
    let mut result = RankedDocumentArticle::compile_query_filter("title:diesel OR note:diesel")
        .unwrap()
        .unwrap();
    let rank = result
        .take_signal("relevance")
        .unwrap()
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let stored_scores = articles::table
        .select((articles::id, rank))
        .order(articles::id)
        .load::<(i32, f32)>(&mut conn)
        .unwrap();
    assert_eq!(scores, stored_scores);
}

#[test]
fn finalized_text_scores_are_not_deduplicated_as_single_leaf_contributions() {
    // An adapter may return signals produced by another compiler. A completed
    // composite must not retain the identity of only its first leaf.
    let mut compiled = Article::compile_query_filter("title:(diesel OR engine)")
        .unwrap()
        .unwrap();
    let leaf =
        postgres::text_query(articles::title, "english", &SearchTerm::exact("diesel")).unwrap();
    compiled.signals.extend(leaf.signals);
    compiled.signals = diesel_aip_160::search::finish_signals(compiled.signals);
    assert_eq!(compiled.signals.len(), 2);
    assert!(compiled.ordering(&["relevance"]).is_err());
    let composite = compiled
        .signals
        .remove(0)
        .into_expression::<diesel::sql_types::Float>()
        .unwrap();
    let query = articles::table.select(composite);
    let sql = diesel::debug_query::<diesel::pg::Pg, _>(&query).to_string();
    assert_eq!(sql.matches("ts_rank_cd").count(), 2);
    assert!(sql.contains("engine"));
}

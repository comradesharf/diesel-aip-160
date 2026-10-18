//! Prefix dialect, typed autocomplete, and PostgreSQL index eligibility.
#![cfg(feature = "postgres")]
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, parser::Expression, search::SearchTerm};

diesel::table! { routes (id) { id -> Integer, long_name -> Nullable<Text>, code -> Nullable<Text> } }
#[derive(Aip160Filter)]
#[diesel(table_name = routes)]
#[allow(dead_code)]
struct Route {
    id: i32,
    #[aip160(search(postgres, configuration = "simple"))]
    long_name: Option<String>,
    #[aip160(search(postgres, configuration = "simple"))]
    code: Option<String>,
}

fn sql(source: &str) -> String {
    let p = Route::compile_filter(source).unwrap().unwrap();
    diesel::debug_query::<diesel::pg::Pg, _>(&routes::table.filter(p).select(routes::id))
        .to_string()
}
#[test]
fn prefix_sql_keeps_index_expression_and_binds_only_literal_text() {
    for source in [
        "long_name:s*",
        "long_name:sen*",
        "code:T7*",
        "sen*",
        "long_name:(sen* OR NOT kuala*)",
    ] {
        let query = sql(source);
        let statement = query.split_once("-- binds:").unwrap().0;
        assert!(
            statement.contains("to_tsvector('simple'::regconfig, \"routes\".\""),
            "{query}"
        );
        assert!(
            statement.contains(
                "::tsquery FROM unnest(tsvector_to_array(to_tsvector('simple'::regconfig, $"
            ),
            "{query}"
        );
        assert!(statement.contains("':*'"));
        assert!(!statement.contains(" LIKE "));
        assert!(!statement.contains("COALESCE(to_tsvector"));
    }
    assert!(sql("long_name:sentral").contains("plainto_tsquery"));
    for source in [r#"long_name:"sen*""#, r#"long_name:"sen\*""#] {
        assert!(sql(source).contains("phraseto_tsquery"));
        assert!(!sql(source).contains("':*'"));
    }
    for source in [
        "long_name:*sen",
        "long_name:s*n",
        "long_name:sen**",
        "long_name:!!!*",
        "*",
        r"long_name:sen\*",
    ] {
        assert!(Route::compile_filter(source).is_err(), "{source}");
        assert!(Route::compile_query_filter(source).is_err(), "{source}");
    }
    for source in [
        "long_name:*",
        "NOT long_name:*",
        "long_name=null",
        "long_name=sen*",
        "long_name='sen*'",
    ] {
        assert!(!sql(source).contains("tsquery"), "{source}");
    }
    for text in [
        r"x' | !sentral:*",
        r"foo\bar",
        "吉隆坡",
        "éco",
        "x'); DROP TABLE routes; --",
    ] {
        let expression = Expression::Search {
            field: Some("long_name".into()),
            term: SearchTerm::prefix(text).unwrap(),
        };
        let mut result = Route::compile_search(&expression).unwrap();
        let order = result.ordering(&["relevance"]).unwrap();
        let query = diesel::debug_query::<diesel::pg::Pg, _>(
            &routes::table.filter(result.predicate).order(order),
        )
        .to_string();
        let (statement, binds) = query.split_once("-- binds:").unwrap();
        assert!(!statement.contains(text), "{query}");
        assert_eq!(binds.matches(&format!("{text:?}")).count(), 2, "{query}");
    }
}

#[test]
fn prefix_ranking_preserves_polarity_deduplication_and_distinct_search_kinds() {
    for (source, scores) in [
        ("NOT long_name:sen*", 0),
        ("NOT (NOT long_name:sen*)", 1),
        ("long_name:sen* OR long_name:sen*", 1),
        ("long_name:sen OR long_name:sen*", 2),
        ("long_name:(sen* AND NOT kuala*)", 1),
        ("sen*", 2),
    ] {
        let mut result = Route::compile_query_filter(source).unwrap().unwrap();
        if scores == 0 {
            assert!(result.signals.is_empty());
            continue;
        }
        let order = result.ordering(&["relevance"]).unwrap();
        let query = diesel::debug_query::<diesel::pg::Pg, _>(
            &routes::table
                .order(order)
                .then_order_by(routes::id)
                .limit(5)
                .offset(2),
        )
        .to_string();
        assert_eq!(
            query.matches("ts_rank_cd").count(),
            scores,
            "{source}: {query}"
        );
        assert!(!query.contains("kuala"));
        assert!(query.contains("DESC, \"routes\".\"id\" LIMIT"), "{query}");
    }
}

#[cfg(feature = "postgres-integration")]
#[test]
#[ignore = "requires PostgreSQL; set AIP160_TEST_DATABASE_URL"]
fn prefix_execution_autocomplete_and_index_eligibility() {
    use diesel::connection::SimpleConnection;
    let mut conn =
        diesel::pg::PgConnection::establish(&std::env::var("AIP160_TEST_DATABASE_URL").unwrap())
            .unwrap();
    conn.batch_execute("BEGIN;
        CREATE TEMP TABLE routes (id integer PRIMARY KEY, long_name text, code text);
        INSERT INTO routes VALUES (1,'Kuala Sentral','T710'), (2,'Sentral Kuala','T7'),
        (3,'Presen station','X1'), (4,NULL,NULL), (5,'sen','S1'),
        (6,'École 吉隆坡','U1'), (7,'O''Reilly foo\\bar','Q1');
        INSERT INTO routes SELECT i, 'ordinary route ' || i, 'R' || i FROM generate_series(100,3100) AS i;
        CREATE INDEX routes_name_fts ON routes USING GIN (to_tsvector('simple'::regconfig,long_name));
        CREATE INDEX routes_code_fts ON routes USING GIN (to_tsvector('simple'::regconfig,code));
        ANALYZE routes;").unwrap();
    let mut ids = |source: &str| {
        routes::table
            .filter(Route::compile_filter(source).unwrap().unwrap())
            .select(routes::id)
            .order(routes::id)
            .load::<i32>(&mut conn)
            .unwrap()
    };
    for (source, expected) in [
        ("long_name:s*", vec![1, 2, 3, 5]),
        ("long_name:sen*", vec![1, 2, 5]),
        ("sen*", vec![1, 2, 5]),
        ("long_name:sentral", vec![1, 2]),
        ("long_name:sen", vec![5]),
        ("code:T7*", vec![1, 2]),
        ("long_name:'Kuala Sentral'", vec![1]),
        ("long_name:(kuala AND sen*)", vec![1, 2]),
        ("NOT long_name:sen* AND id<100", vec![3, 6, 7]),
        ("NOT (NOT long_name:sen*)", vec![1, 2, 5]),
        ("long_name:(sen* OR idontexist)", vec![1, 2, 5]),
        ("long_name=null", vec![4]),
        ("long_name='Kuala*'", vec![1]),
        ("long_name:éco*", vec![6]),
        ("long_name:吉*", vec![6]),
        ("long_name:'!!!'", vec![]),
        (r#"long_name:"sen\*""#, vec![5]),
    ] {
        assert_eq!(ids(source), expected, "{source}");
    }
    for input in ["kuala sen", "kuala sen  ", "kuala !!! sen"] {
        let expression = diesel_aip_160::search::autocomplete_field("long_name", input)
            .unwrap()
            .unwrap();
        let mut compiled = Route::compile_search(&expression).unwrap();
        let order = compiled.ordering(&["relevance"]).unwrap();
        let first = routes::table
            .filter(compiled.predicate)
            .select(routes::id)
            .order(order)
            .then_order_by(routes::id)
            .limit(1)
            .load::<i32>(&mut conn)
            .unwrap();
        assert_eq!(first, vec![1]);
    }
    for (input, expected) in [
        (r"x' | !sentral:*", vec![]),
        (r"foo\bar", vec![7]),
        ("O'Rei", vec![7]),
    ] {
        let expression = Expression::Search {
            field: Some("long_name".into()),
            term: SearchTerm::prefix(input).unwrap(),
        };
        let mut compiled = Route::compile_search(&expression).unwrap();
        let rank = compiled
            .take_signal("relevance")
            .unwrap()
            .into_expression::<diesel::sql_types::Float>()
            .unwrap();
        let rows = routes::table
            .filter(compiled.predicate)
            .select((routes::id, rank))
            .order(routes::id)
            .load::<(i32, f32)>(&mut conn)
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            expected,
            "{input}"
        );
        assert!(rows.iter().all(|r| r.1 > 0.0));
    }
    let mut ranked = |source: &str| {
        let mut compiled = Route::compile_query_filter(source).unwrap().unwrap();
        let rank = compiled
            .take_signal("relevance")
            .unwrap()
            .into_expression::<diesel::sql_types::Float>()
            .unwrap();
        routes::table
            .filter(compiled.predicate)
            .select((routes::id, rank))
            .order(routes::id)
            .load::<(i32, f32)>(&mut conn)
            .unwrap()
    };
    let scores = ranked("long_name:sen* OR id=4");
    assert_eq!(
        scores.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![1, 2, 4, 5]
    );
    assert!(scores.iter().filter(|r| r.0 != 4).all(|r| r.1 > 0.0));
    assert_eq!(scores[2].1, 0.0); // Positive branch may be unknown for a NULL row.
    assert_eq!(scores, ranked("long_name:sen* OR long_name:sen* OR id=4"));
    assert_eq!(scores, ranked("NOT (NOT long_name:sen*) OR id=4"));
    assert_eq!(
        scores,
        ranked("long_name:sen* OR (id=4 AND NOT long_name:kuala*) OR id=4")
    );
    let expression = diesel_aip_160::search::autocomplete_field("long_name", "kuala sen")
        .unwrap()
        .unwrap();
    let mut compiled = Route::compile_search(&expression).unwrap();
    let order = compiled.ordering(&["relevance"]).unwrap();
    assert_eq!(
        routes::table
            .filter(compiled.predicate)
            .select(routes::id)
            .order(order)
            .then_order_by(routes::id)
            .limit(1)
            .offset(1)
            .load::<i32>(&mut conn)
            .unwrap(),
        vec![2]
    );
    #[derive(QueryableByName)]
    struct Plan {
        #[diesel(sql_type = diesel::sql_types::Json, column_name = "QUERY PLAN")]
        plan: serde_json::Value,
    }
    fn uses_index(value: &serde_json::Value, name: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.get("Index Name").and_then(|v| v.as_str()) == Some(name)
                    || map.values().any(|v| uses_index(v, name))
            }
            serde_json::Value::Array(items) => items.iter().any(|v| uses_index(v, name)),
            _ => false,
        }
    }
    // EXPLAIN the actual generated predicate, preserving parameter binding.
    for (source, bind, index) in [
        ("long_name:sen*", "sen", "routes_name_fts"),
        ("long_name:s*", "s", "routes_name_fts"),
        ("code:T7*", "T7", "routes_code_fts"),
    ] {
        let debug = sql(source);
        let statement = debug.split_once("-- binds:").unwrap().0;
        let explain = format!("EXPLAIN (FORMAT JSON) {statement}");
        let natural = diesel::sql_query(&explain)
            .bind::<diesel::sql_types::Text, _>(bind)
            .get_result::<Plan>(&mut conn)
            .unwrap();
        eprintln!(
            "{source}: natural root {}",
            natural.plan[0]["Plan"]["Node Type"]
        );
        conn.batch_execute("SET LOCAL enable_seqscan=off").unwrap();
        let eligible = diesel::sql_query(&explain)
            .bind::<diesel::sql_types::Text, _>(bind)
            .get_result::<Plan>(&mut conn)
            .unwrap();
        assert!(
            uses_index(&eligible.plan, index),
            "{source}: {}",
            eligible.plan
        );
        conn.batch_execute("SET LOCAL enable_seqscan=on").unwrap();
        eprintln!("{source}: eligible index {index}");
    }
    conn.batch_execute("ROLLBACK").unwrap();
}

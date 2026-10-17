#![cfg(feature = "postgres")]

// Deliberately no Diesel prelude or std::io::Write import.
use diesel::pg::{Pg, PgMetadataLookup, PgTypeMetadata};
use diesel::query_builder::BindCollector;
use diesel::query_builder::bind_collector::RawBytesBindCollector;
use diesel_aip_160::Aip160PgEnum;

#[derive(diesel::sql_types::SqlType, diesel::query_builder::QueryId)]
#[diesel(postgres_type(name = "aip160_route_type"))]
pub struct RouteTypeSql;

// Stands in for a non-Copy enum owned by another crate.
#[derive(Debug, PartialEq)]
enum ExternalRoute {
    Bus,
    Extended(i16),
    Other(String),
}

fn decode(code: i16) -> diesel::deserialize::Result<ExternalRoute> {
    match code {
        3 => Ok(ExternalRoute::Bus),
        100 => Ok(ExternalRoute::Extended(code)),
        _ => Err(format!("Unknown GTFS route type: {code}").into()),
    }
}

fn encode(route: &ExternalRoute) -> diesel::deserialize::Result<i16> {
    match route {
        ExternalRoute::Bus => Ok(3),
        ExternalRoute::Extended(100) => Ok(100),
        _ => Err("unsupported route".into()),
    }
}

#[derive(
    Debug,
    PartialEq,
    diesel::expression::AsExpression,
    diesel::deserialize::FromSqlRow,
    Aip160PgEnum,
)]
#[diesel(sql_type = RouteTypeSql)]
#[aip160(value_type = i16, from_value = decode, to_value = encode)]
struct Route(ExternalRoute);

diesel::table! {
    use diesel::sql_types::*;
    use super::RouteTypeSql;
    enum_routes (id) {
        id -> Integer,
        name -> Text,
        kind -> RouteTypeSql,
        optional -> Nullable<RouteTypeSql>,
    }
}

type RouteAlias = Route;

#[derive(diesel_aip_160::Aip160Filter)]
#[diesel(table_name = enum_routes)]
#[allow(dead_code)]
struct Model {
    id: i32,
    name: String,
    kind: RouteAlias,
    optional: Option<crate::Route>,
}

struct RejectAdapter;
impl diesel_aip_160::diesel_helpers::FieldAdapter<enum_routes::kind> for RejectAdapter {
    fn compare(
        _: enum_routes::kind,
        _: diesel_aip_160::compiler::ComparisonOperator,
        _: &diesel_aip_160::parser::Value,
    ) -> diesel_aip_160::anyhow::Result<diesel_aip_160::diesel_helpers::Predicate<enum_routes::table>>
    {
        diesel_aip_160::anyhow::bail!("explicit adapter selected")
    }
}

#[derive(diesel_aip_160::Aip160Filter)]
#[diesel(table_name = enum_routes)]
#[allow(dead_code)]
struct OverrideModel {
    #[aip160(adapter = RejectAdapter)]
    kind: Route,
}

fn sql(filter: &str) -> String {
    use diesel::QueryDsl;
    let predicate = Model::compile_filter(filter).unwrap().unwrap();
    diesel::debug_query::<Pg, _>(&enum_routes::table.filter(predicate)).to_string()
}

#[test]
fn models_reuse_enum_conversions_without_field_attributes() {
    for field in ["kind", "optional"] {
        for operator in ["=", "!=", "<", "<=", ">", ">=", ":"] {
            let query = sql(&format!("{field}{operator}100"));
            let expected = if operator == ":" { "=" } else { operator };
            assert!(
                query.contains(&format!("\"{field}\" {expected} $1")),
                "{query}"
            );
            assert!(
                query.ends_with("-- binds: [Route(Extended(100))]"),
                "{query}"
            );
        }
    }
    assert!(sql("kind='3'").ends_with("-- binds: [Route(Bus)]"));
    assert!(sql("optional=null").contains("IS NULL"));
    assert!(sql("optional!=null").contains("IS NOT NULL"));
    assert!(sql("optional:*").contains("IS NOT NULL"));
    assert!(sql("kind:*").contains("IS NOT NULL"));

    use diesel_aip_160::compiler::Aip160FilterCompiler;
    assert_eq!(ModelFilterCompiler.text_fields(), ["name"]);
    assert!(ModelFilterCompiler.ilike("kind", "3".into()).is_err());
    let global = sql("Bus");
    assert!(global.contains("lower(\"enum_routes\".\"name\")"));
    assert!(!global.contains("lower(\"enum_routes\".\"kind\")"));
    assert!(
        OverrideModel::compile_filter("kind=3")
            .err()
            .unwrap()
            .to_string()
            .contains("explicit adapter selected")
    );
}

#[test]
fn enum_filters_reject_invalid_literals_and_paths_before_sql() {
    for filter in [
        "kind=999",
        "kind=40000",
        "kind=invalid",
        "kind='3); DROP TABLE enum_routes; --'",
        "kind=null",
        "kind!=null",
        "kind.x=3",
        "optional>null",
        "optional:null",
        "optional='null'",
    ] {
        assert!(Model::compile_filter(filter).is_err(), "{filter}");
    }
}

struct Metadata;
impl PgMetadataLookup for Metadata {
    fn lookup_type(&mut self, name: &str, _: Option<&str>) -> PgTypeMetadata {
        assert_eq!(name, "aip160_route_type");
        PgTypeMetadata::new(42, 43)
    }
}

#[test]
fn generated_serializer_accepts_non_copy_values_and_propagates_errors() {
    let mut binds = RawBytesBindCollector::<Pg>::new();
    for value in [
        Route(ExternalRoute::Bus),
        Route(ExternalRoute::Extended(100)),
    ] {
        binds
            .push_bound_value::<RouteTypeSql, _>(&value, &mut Metadata)
            .unwrap();
    }
    let error = binds
        .push_bound_value::<RouteTypeSql, _>(
            &Route(ExternalRoute::Other("unknown".into())),
            &mut Metadata,
        )
        .unwrap_err();
    assert!(error.to_string().contains("unsupported route"));
}

#[test]
fn wrapper_supports_nullable_typed_bind_expressions() {
    // Compile-check both Diesel derives in addition to the generated codecs.
    fn assert_expression<
        T: diesel::expression::AsExpression<diesel::sql_types::Nullable<RouteTypeSql>>,
    >() {
    }
    fn assert_row<T: diesel::deserialize::FromSqlRow<RouteTypeSql, Pg>>() {}
    assert_expression::<Option<Route>>();
    assert_row::<Route>();
}

#[cfg(feature = "postgres-integration")]
#[test]
#[ignore = "requires AIP160_TEST_DATABASE_URL and a PostgreSQL server"]
fn postgres_enum_labels_round_trip_and_invalid_labels_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, RunQueryDsl};
    let url = std::env::var("AIP160_TEST_DATABASE_URL")?;
    let mut connection = diesel::pg::PgConnection::establish(&url)?;
    connection.batch_execute(
        "CREATE TYPE pg_temp.aip160_route_type AS ENUM ('100', '3', '999', '40000', 'invalid');
         CREATE TEMP TABLE enum_routes (
             id INTEGER PRIMARY KEY, name TEXT NOT NULL,
             kind pg_temp.aip160_route_type NOT NULL,
             optional pg_temp.aip160_route_type
         );
         INSERT INTO enum_routes VALUES
             (1, 'Bus', '3', NULL), (2, 'Rail', '100', '3'), (3, 'Bus', '3', '100');",
    )?;
    for route in [
        Route(ExternalRoute::Bus),
        Route(ExternalRoute::Extended(100)),
    ] {
        let decoded =
            diesel::select(diesel::dsl::sql::<RouteTypeSql>("").bind::<RouteTypeSql, _>(&route))
                .get_result::<Route>(&mut connection)?;
        assert_eq!(decoded, route);
    }
    for label in ["999", "40000", "invalid"] {
        let query = format!("'{label}'::pg_temp.aip160_route_type");
        assert!(
            diesel::select(diesel::dsl::sql::<RouteTypeSql>(&query))
                .get_result::<Route>(&mut connection)
                .is_err()
        );
    }
    use diesel::QueryDsl;
    for (filter, expected) in [
        ("kind=3", vec![1, 3]),
        ("kind:100", vec![2]),
        ("kind!=3", vec![2]),
        // PostgreSQL enum declaration order deliberately differs from numeric order.
        ("kind<3", vec![2]),
        ("kind<=100", vec![2]),
        ("kind>100", vec![1, 3]),
        ("kind>=3", vec![1, 3]),
        ("kind=(3 OR 100)", vec![1, 2, 3]),
        ("kind:*", vec![1, 2, 3]),
        ("optional=null", vec![1]),
        ("optional!=null", vec![2, 3]),
        ("optional:*", vec![2, 3]),
        ("NOT optional=3", vec![3]),
        ("optional=null OR optional=100", vec![1, 3]),
        ("kind=3 AND name:Bus", vec![1, 3]),
        ("Bus", vec![1, 3]),
    ] {
        let predicate = Model::compile_filter(filter)?.unwrap();
        let ids = enum_routes::table
            .filter(predicate)
            .select(enum_routes::id)
            .order(enum_routes::id)
            .load::<i32>(&mut connection)?;
        assert_eq!(ids, expected, "{filter}");
    }
    Ok(())
}

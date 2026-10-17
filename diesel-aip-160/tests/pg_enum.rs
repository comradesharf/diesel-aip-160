#![cfg(feature = "postgres")]

// Deliberately no Diesel prelude or std::io::Write import.
use diesel::pg::{Pg, PgMetadataLookup, PgTypeMetadata};
use diesel::query_builder::BindCollector;
use diesel::query_builder::bind_collector::RawBytesBindCollector;
use diesel_aip_160::Aip160PgEnum;

#[derive(diesel::sql_types::SqlType, diesel::query_builder::QueryId)]
#[diesel(postgres_type(name = "aip160_route_type"))]
struct RouteTypeSql;

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
        "CREATE TYPE pg_temp.aip160_route_type AS ENUM ('3', '100', '999', '40000', 'invalid');",
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
    Ok(())
}

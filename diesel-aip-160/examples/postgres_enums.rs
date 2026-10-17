//! Derived PostgreSQL enum codecs and automatic filtering, with application
//! adapters for numeric route ordering and protobuf-to-GTFS mapping.
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable};
use diesel_aip_160::anyhow::{Result, bail, ensure};
use diesel_aip_160::compiler::ComparisonOperator;
use diesel_aip_160::diesel_helpers::{FieldAdapter, Predicate};
use diesel_aip_160::parser::Value;
use diesel_aip_160::{Aip160Filter, Aip160PgEnum};

#[derive(diesel::sql_types::SqlType, diesel::query_builder::QueryId)]
#[diesel(postgres_type(name = "route_type"))]
pub struct RouteTypeSql;
#[derive(diesel::sql_types::SqlType, diesel::query_builder::QueryId)]
#[diesel(postgres_type(name = "pickup_drop_off"))]
pub struct PickupSql;

// Declaration order may be ('1702', '3', '100'); the adapter compares codes.
diesel::table! {
    use diesel::sql_types::*;
    use super::{RouteTypeSql, PickupSql};
    routes (id) {
        id -> Integer,
        small -> Nullable<Integer>,
        wide -> BigInt,
        route_type -> RouteTypeSql,
        continuous_pickup -> PickupSql,
        continuous_drop_off -> Nullable<PickupSql>,
        order -> Nullable<BigInt>,
    }
}

// These domain enums stand in for types owned by a dependency. Local wrappers
// let the application implement Diesel codecs without changing that dependency.
#[derive(Debug)]
enum RouteKind {
    Bus,
    Rail,
    HorseDrawnCarriage,
}

fn route_from_code(code: i16) -> diesel::deserialize::Result<RouteKind> {
    match code {
        3 => Ok(RouteKind::Bus),
        100 => Ok(RouteKind::Rail),
        1702 => Ok(RouteKind::HorseDrawnCarriage),
        _ => Err(format!("unsupported route_type code: {code}").into()),
    }
}

fn route_to_code(route: &RouteKind) -> diesel::deserialize::Result<i16> {
    Ok(match route {
        RouteKind::Bus => 3,
        RouteKind::Rail => 100,
        RouteKind::HorseDrawnCarriage => 1702,
    })
}

#[derive(
    Debug, diesel::expression::AsExpression, diesel::deserialize::FromSqlRow, Aip160PgEnum,
)]
#[diesel(sql_type = RouteTypeSql)]
#[aip160(value_type = i16, from_value = route_from_code, to_value = route_to_code)]
struct RouteType(RouteKind);

#[derive(Debug)]
enum PickupKind {
    Regular,
    Unavailable,
    PhoneAgency,
    CoordinateWithDriver,
}

fn pickup_from_code(code: i16) -> diesel::deserialize::Result<PickupKind> {
    match code {
        0 => Ok(PickupKind::Regular),
        1 => Ok(PickupKind::Unavailable),
        2 => Ok(PickupKind::PhoneAgency),
        3 => Ok(PickupKind::CoordinateWithDriver),
        _ => Err(format!("unsupported GTFS pickup code: {code}").into()),
    }
}

fn pickup_to_code(pickup: &PickupKind) -> diesel::deserialize::Result<i16> {
    Ok(match pickup {
        PickupKind::Regular => 0,
        PickupKind::Unavailable => 1,
        PickupKind::PhoneAgency => 2,
        PickupKind::CoordinateWithDriver => 3,
    })
}

#[derive(
    Debug, diesel::expression::AsExpression, diesel::deserialize::FromSqlRow, Aip160PgEnum,
)]
#[diesel(sql_type = PickupSql)]
#[aip160(value_type = i16, from_value = pickup_from_code, to_value = pickup_to_code)]
struct Pickup(PickupKind);

// No field adapters are needed for stored GTFS labels. Ordering follows the
// PostgreSQL enum declaration, even when the labels look like numbers.
#[derive(Aip160Filter)]
#[diesel(table_name = routes)]
#[allow(dead_code)]
struct NativeRoute {
    route_type: RouteType,
    continuous_pickup: Pickup,
    continuous_drop_off: Option<Pickup>,
}

struct NumericRoute;
impl FieldAdapter<routes::route_type> for NumericRoute {
    fn compare(
        _: routes::route_type,
        operator: ComparisonOperator,
        value: &Value,
    ) -> Result<Predicate<routes::table>> {
        ensure!(
            !value.quoted,
            "route_type requires an unquoted numeric code"
        );
        let code: i64 = value.text.parse()?;
        ensure!(
            [3, 100, 1702].contains(&code),
            "unsupported route_type code"
        );
        // Trusted application SQL only. Never concatenate filter input here.
        // PostgreSQL enum -> text -> bigint explicitly selects numeric ordering.
        let numeric = diesel::dsl::sql::<Nullable<BigInt>>(
            "CAST(CAST(\"routes\".\"route_type\" AS TEXT) AS BIGINT)",
        );
        Ok(match operator {
            ComparisonOperator::Equal => Box::new(numeric.eq(code).assume_not_null()),
            ComparisonOperator::NotEqual => Box::new(numeric.ne(code).assume_not_null()),
            ComparisonOperator::LessThan => Box::new(numeric.lt(code).assume_not_null()),
            ComparisonOperator::LessThanOrEqual => Box::new(numeric.le(code).assume_not_null()),
            ComparisonOperator::GreaterThan => Box::new(numeric.gt(code).assume_not_null()),
            ComparisonOperator::GreaterThanOrEqual => Box::new(numeric.ge(code).assume_not_null()),
        })
    }
}

// Only these literals are accepted; protobuf 1..=4 maps to GTFS labels 0..=3.
fn mapped_pickup(value: &Value) -> Result<Pickup> {
    ensure!(!value.quoted, "pickup requires an unquoted protobuf value");
    let code = match value.text.as_str() {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        "4" => 3,
        _ => bail!("pickup requires a protobuf value from 1 through 4"),
    };
    Ok(Pickup(
        pickup_from_code(code).map_err(diesel_aip_160::anyhow::Error::from_boxed)?,
    ))
}

struct ProtobufPickup;
macro_rules! pickup_adapter {
    ($column:ident) => {
        impl FieldAdapter<routes::$column> for ProtobufPickup {
            fn compare(
                column: routes::$column,
                operator: ComparisonOperator,
                value: &Value,
            ) -> Result<Predicate<routes::table>> {
                let column = column.nullable();
                if !value.quoted && value.text == "null" {
                    return Ok(match operator {
                        ComparisonOperator::Equal => Box::new(column.is_null()),
                        ComparisonOperator::NotEqual => Box::new(column.is_not_null()),
                        _ => bail!("null supports only = and != comparisons"),
                    });
                }
                let mapped = mapped_pickup(value)?;
                // Typed PostgreSQL enum bind; ordering is deliberately unsupported.
                Ok(match operator {
                    ComparisonOperator::Equal => Box::new(column.eq(mapped).assume_not_null()),
                    ComparisonOperator::NotEqual => Box::new(column.ne(mapped).assume_not_null()),
                    _ => bail!("pickup supports only equality and inequality"),
                })
            }
        }
    };
}
pickup_adapter!(continuous_pickup);
pickup_adapter!(continuous_drop_off);

// Explicit adapters override the wrappers' automatic filtering for this model.
#[derive(Aip160Filter)]
#[diesel(table_name = routes)]
#[allow(dead_code)]
struct Route {
    id: i32,
    small: Option<i32>,
    wide: i64,
    #[aip160(adapter = NumericRoute)]
    route_type: RouteType,
    #[aip160(adapter = ProtobufPickup)]
    continuous_pickup: Pickup,
    #[aip160(adapter = ProtobufPickup)]
    continuous_drop_off: Option<Pickup>,
    order: Option<i64>,
}

fn sql(filter: &str) -> Result<String> {
    let predicate = Route::compile_filter(filter)?.unwrap();
    Ok(diesel::debug_query::<diesel::pg::Pg, _>(&routes::table.filter(predicate)).to_string())
}

fn native_sql(filter: &str) -> Result<String> {
    let predicate = NativeRoute::compile_filter(filter)?.unwrap();
    Ok(diesel::debug_query::<diesel::pg::Pg, _>(&routes::table.filter(predicate)).to_string())
}

fn main() -> Result<()> {
    println!(
        "Stored GTFS labels (PostgreSQL enum order):\n{}",
        native_sql("route_type>=100 AND continuous_pickup=1 AND continuous_drop_off!=null")?
    );
    println!(
        "Numeric route order and protobuf pickup values:\n{}",
        sql("route_type>=100 AND continuous_pickup=2 AND order<=4294967295")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_enums_filter_stored_labels_without_adapters() {
        for operator in ["=", "!=", "<", "<=", ">", ">=", ":"] {
            let query = native_sql(&format!("route_type{operator}100")).unwrap();
            let operator = if operator == ":" { "=" } else { operator };
            assert!(
                query.contains(&format!("\"route_type\" {operator} $1")),
                "{query}"
            );
            assert!(!query.contains("AS BIGINT"), "{query}");
            assert!(query.ends_with("-- binds: [RouteType(Rail)]"), "{query}");
        }
        assert!(
            native_sql("route_type='3'")
                .unwrap()
                .ends_with("-- binds: [RouteType(Bus)]")
        );
        // Automatic filtering uses stored labels; the protobuf adapter maps 1 to 0.
        assert!(
            native_sql("continuous_pickup=1")
                .unwrap()
                .ends_with("-- binds: [Pickup(Unavailable)]")
        );
        assert!(
            sql("continuous_pickup=1")
                .unwrap()
                .ends_with("-- binds: [Pickup(Regular)]")
        );
        assert!(
            native_sql("continuous_drop_off=null")
                .unwrap()
                .contains("IS NULL")
        );
        assert!(
            native_sql("continuous_drop_off:*")
                .unwrap()
                .contains("IS NOT NULL")
        );
        for filter in [
            "route_type=4",
            "route_type=null",
            "route_type.x=3",
            "continuous_pickup=4",
            "continuous_drop_off>null",
        ] {
            assert!(NativeRoute::compile_filter(filter).is_err(), "{filter}");
        }
    }

    /// Uses only session-local types/tables on an explicitly provided test database.
    #[cfg(feature = "postgres-integration")]
    #[test]
    #[ignore = "requires AIP160_TEST_DATABASE_URL and a PostgreSQL server"]
    fn postgres_executes_numeric_ordering_mapping_and_nulls() -> Result<()> {
        use diesel::connection::SimpleConnection;
        let url = std::env::var("AIP160_TEST_DATABASE_URL")?;
        let mut connection = diesel::pg::PgConnection::establish(&url)?;
        connection.batch_execute(
            r#"
            CREATE TYPE pg_temp.route_type AS ENUM ('1702', '3', '100');
            CREATE TYPE pg_temp.pickup_drop_off AS ENUM ('0', '1', '2', '3');
            CREATE TEMP TABLE routes (
                id INTEGER PRIMARY KEY,
                small INTEGER,
                wide BIGINT NOT NULL,
                route_type pg_temp.route_type NOT NULL,
                continuous_pickup pg_temp.pickup_drop_off NOT NULL,
                continuous_drop_off pg_temp.pickup_drop_off,
                "order" BIGINT
            );
            INSERT INTO routes VALUES
                (1, -2147483648, -9223372036854775808, '3', '0', NULL, NULL),
                (2, NULL, 4294967295, '100', '1', '2', 4294967295),
                (3, 2147483647, 9223372036854775807, '1702', '3', '0', 9223372036854775807);
        "#,
        )?;
        // Native enum ordering is deliberately different: 1702 precedes 3.
        let native = routes::table
            .order(routes::route_type)
            .select(routes::id)
            .load::<i32>(&mut connection)?;
        assert_eq!(native, [3, 1, 2]);
        for (filter, expected) in [
            ("route_type<100", vec![1, 3]),
            ("route_type>=100", vec![2]),
            ("continuous_pickup=1", vec![2]),
            ("continuous_drop_off=null", vec![1]),
            ("continuous_drop_off:*", vec![2, 3]),
        ] {
            let predicate = NativeRoute::compile_filter(filter)?.unwrap();
            let actual = routes::table
                .filter(predicate)
                .select(routes::id)
                .order(routes::id)
                .load::<i32>(&mut connection)?;
            assert_eq!(actual, expected, "native: {filter}");
        }
        // The generated FromSql codecs decode the same domain values used by filters.
        let (route, pickup, drop_off) = routes::table
            .find(2)
            .select((
                routes::route_type,
                routes::continuous_pickup,
                routes::continuous_drop_off,
            ))
            .first::<(RouteType, Pickup, Option<Pickup>)>(&mut connection)?;
        assert!(matches!(route.0, RouteKind::Rail));
        assert!(matches!(pickup.0, PickupKind::Unavailable));
        assert!(matches!(drop_off, Some(Pickup(PickupKind::PhoneAgency))));
        for (filter, expected) in [
            ("small=-2147483648", vec![1]),
            ("small=2147483647", vec![3]),
            ("small=null", vec![2]),
            ("small!=null", vec![1, 3]),
            ("NOT small<0", vec![3]),
            ("small=null OR small<=-2147483648", vec![1, 2]),
            ("wide=-9223372036854775808", vec![1]),
            ("wide>4294967295", vec![3]),
            ("route_type<100", vec![1]),
            ("route_type<=100", vec![1, 2]),
            ("route_type>100", vec![3]),
            ("route_type>=100", vec![2, 3]),
            ("route_type=1702", vec![3]),
            ("route_type!=100", vec![1, 3]),
            ("continuous_pickup=2", vec![2]),
            ("continuous_drop_off=3", vec![2]),
            ("continuous_drop_off=null", vec![1]),
            ("NOT continuous_drop_off=1", vec![2]),
            ("order=4294967295", vec![2]),
            ("order=null OR order>=4294967295", vec![1, 2, 3]),
            ("NOT order<9223372036854775807", vec![3]),
            ("route_type>=100 AND continuous_pickup=(2 OR 4)", vec![2, 3]),
        ] {
            let predicate = Route::compile_filter(filter)?.unwrap();
            let actual = routes::table
                .filter(predicate)
                .select(routes::id)
                .order(routes::id)
                .load::<i32>(&mut connection)?;
            assert_eq!(actual, expected, "{filter}");
        }
        Ok(())
    }

    #[test]
    fn numeric_ordering_uses_cast_and_bound_bigints() {
        for code in [3, 100, 1702] {
            for operator in ["=", "!=", "<", "<=", ">", ">="] {
                let query = sql(&format!("route_type{operator}{code}")).unwrap();
                let statement = query.split(" -- binds:").next().unwrap();
                assert!(statement.contains("AS TEXT) AS BIGINT)"), "{query}");
                assert!(statement.contains(&format!("{operator} $1")), "{query}");
                assert!(query.ends_with(&format!("-- binds: [{code}]")), "{query}");
            }
        }
    }

    #[test]
    fn enum_mapping_binds_typed_stored_labels() {
        for (literal, variant) in [
            (1, "Regular"),
            (2, "Unavailable"),
            (3, "PhoneAgency"),
            (4, "CoordinateWithDriver"),
        ] {
            for field in ["continuous_pickup", "continuous_drop_off"] {
                let query = sql(&format!("{field}={literal}")).unwrap();
                assert!(query.contains(&format!("\"{field}\" = $1")), "{query}");
                assert!(
                    query.ends_with(&format!("-- binds: [Pickup({variant})]")),
                    "{query}"
                );
            }
        }
        assert!(sql("continuous_drop_off=null").unwrap().contains("IS NULL"));
        assert!(
            sql("continuous_drop_off:*")
                .unwrap()
                .contains("IS NOT NULL")
        );
        assert!(sql("continuous_pickup!=4").unwrap().contains(" != $1"));
        let query =
            sql("(route_type=3 OR route_type>=100) AND NOT continuous_pickup=1 AND order=null")
                .unwrap();
        assert!(query.contains("IS NULL") && query.contains("NOT"));
        assert!(sql("route_type:*").unwrap().ends_with("-- binds: [true]"));
    }

    #[test]
    fn invalid_enum_input_is_rejected_before_sql() {
        for filter in [
            "route_type=4",
            "route_type=9223372036854775808",
            "route_type='1702'",
            "route_type='3); DROP TABLE routes; --'",
            "route_type=null",
            "route_type.x=3",
            "continuous_pickup=0",
            "continuous_pickup=5",
            "continuous_pickup='1'",
            "continuous_drop_off>null",
            "continuous_drop_off='null'",
            "continuous_pickup>1",
            "continuous_pickup=null",
            "continuous_pickup='*'",
        ] {
            assert!(Route::compile_filter(filter).is_err(), "{filter}");
        }
    }
}

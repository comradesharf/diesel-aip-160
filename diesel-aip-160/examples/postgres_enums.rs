//! Application adapters: numeric route ordering and protobuf-to-GTFS mapping.
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable};
use diesel_aip_160::Aip160Filter;
use diesel_aip_160::anyhow::{Result, bail, ensure};
use diesel_aip_160::compiler::ComparisonOperator;
use diesel_aip_160::diesel_helpers::{FieldAdapter, Predicate};
use diesel_aip_160::parser::Value;

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

#[derive(Debug)]
struct RouteType;

#[derive(Debug, diesel::expression::AsExpression)]
#[diesel(sql_type = PickupSql)]
struct Pickup(&'static str);

impl diesel::serialize::ToSql<PickupSql, diesel::pg::Pg> for Pickup {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>,
    ) -> diesel::serialize::Result {
        use std::io::Write;
        out.write_all(self.0.as_bytes())?;
        Ok(diesel::serialize::IsNull::No)
    }
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
    Ok(Pickup(match value.text.as_str() {
        "1" => "0",
        "2" => "1",
        "3" => "2",
        "4" => "3",
        _ => bail!("pickup requires a protobuf value from 1 through 4"),
    }))
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

fn main() -> Result<()> {
    println!(
        "{}",
        sql("route_type>=100 AND continuous_pickup=2 AND order<=4294967295")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for (literal, label) in [(1, "0"), (2, "1"), (3, "2"), (4, "3")] {
            for field in ["continuous_pickup", "continuous_drop_off"] {
                let query = sql(&format!("{field}={literal}")).unwrap();
                assert!(query.contains(&format!("\"{field}\" = $1")), "{query}");
                assert!(
                    query.ends_with(&format!("-- binds: [Pickup(\"{label}\")]")),
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

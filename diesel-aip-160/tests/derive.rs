use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, Aip160Jsonb};

table! {
    stops (id) {
        id -> Text,
        name -> Text,
        street -> Nullable<Text>,
        count -> Integer,
        location -> Binary,
        payload -> Jsonb,
    }
}

#[derive(Aip160Jsonb)]
#[allow(dead_code)]
struct Payload {
    #[serde(rename = "01")]
    outbound: Vec<[f64; 2]>,
    details: diesel_aip_160::serde_json::Value,
}

#[derive(Selectable, Aip160Filter)]
#[diesel(table_name = stops)]
#[allow(dead_code)]
struct Stop {
    id: String,
    name: String,
    street: Option<String>,
    count: i32,
    #[aip160(skip)]
    location: Vec<u8>,
    payload: Payload,
}

fn sql(filter: &str) -> String {
    let predicate = Stop::compile_filter(filter).unwrap().unwrap();
    diesel::debug_query::<Pg, _>(&stops::table.filter(predicate)).to_string()
}

#[test]
fn derives_comparisons_from_fields() {
    assert!(sql("id=A").contains("\"stops\".\"id\" = $1"));
    assert!(sql("name:A").contains("\"stops\".\"name\" LIKE $1"));
    assert!(sql("street=null").contains("\"street\" IS NULL"));
    assert!(sql("street:*").contains("\"street\" IS NOT NULL"));
    assert!(sql("count>=5").contains("\"count\" >= $1"));
    assert!(Stop::compile_filter("location=A").is_err());
    assert!(Stop::compile_filter("unknown=A").is_err());
    assert!(Stop::compile_filter("count='5'").is_err());
}

#[test]
fn global_search_uses_only_text_fields() {
    let query = sql("Central");
    for field in ["id", "name", "street"] {
        assert!(query.contains(&format!("\"stops\".\"{field}\" LIKE")));
    }
    assert!(!query.contains("\"count\" LIKE"));
    assert!(!query.contains("\"location\" LIKE"));
}

#[test]
fn jsonb_paths_bind_keys_and_values() {
    let present = sql("payload.outbound:*");
    assert!(present.contains("#> $1"), "{present}");
    assert!(present.contains("IS NOT NULL"), "{present}");
    assert!(present.contains("[\"01\"]"), "{present}");

    let equality = sql("payload.details.enabled=true");
    assert!(equality.contains("#> $1"), "{equality}");
    assert!(equality.contains("= $2"), "{equality}");
    assert!(equality.contains("true"), "{equality}");

    let containment = sql("payload.outbound:'[[101.7,3.1]]'");
    assert!(containment.contains("@>"), "{containment}");
    assert!(containment.contains("Array ["), "{containment}");
    assert!(containment.contains("Number(101.7)"), "{containment}");
    assert!(containment.contains("Number(3.1)"), "{containment}");
    assert!(Stop::compile_filter("payload.01:*").is_err());
}

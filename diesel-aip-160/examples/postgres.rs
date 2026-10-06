//! Run with `cargo run -p diesel-aip-160 --example postgres --features postgres`.
use diesel::{pg::Pg, prelude::*};
use diesel_aip_160::Aip160Filter;

diesel::table! {
    records (id) {
        id -> Integer,
        name -> Text,
        note -> Nullable<Text>,
    }
}

#[derive(Aip160Filter)]
#[allow(dead_code)]
#[diesel(table_name = records)]
struct Record {
    id: i32,
    name: String,
    note: Option<String>,
}

fn main() -> diesel_aip_160::anyhow::Result<()> {
    let predicate = Record::compile_filter("name=Alice AND note:*")?.expect("nonempty filter");
    let query = records::table.filter(predicate);
    println!("{}", diesel::debug_query::<Pg, _>(&query));
    Ok(())
}

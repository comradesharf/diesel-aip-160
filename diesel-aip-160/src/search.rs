//! Opt-in full-text search. Built-in support currently requires PostgreSQL.
//! Applications can implement [`crate::diesel_helpers::SearchAdapter`] on every
//! backend. Database objects and search-document maintenance remain application-owned.

#![cfg_attr(
    not(feature = "postgres"),
    doc = r#"
PostgreSQL built-ins cannot be enabled under another backend:

```compile_fail
use diesel_aip_160::Aip160Filter;
diesel::table! { articles (id) { id -> Integer, title -> Text } }
#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
struct Article {
    #[aip160(search(postgres, configuration = "english"))]
    title: String,
}
```
"#
)]

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "postgres")]
macro_rules! __aip160_require_postgres_search {
    () => {};
}
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "postgres"))]
macro_rules! __aip160_require_postgres_search {
    () => { compile_error!("aip160 search(postgres, ...) requires the postgres backend; built-in MySQL and SQLite full-text search is unsupported; use search_adapter instead"); };
}

/// Complete built-in search signals after Boolean polarity has been resolved.
/// Called by generated compilers; custom compiler implementations own their policy.
#[doc(hidden)]
pub fn finish_signals<T>(
    signals: Vec<crate::ranking::RankingSignal<T>>,
) -> Vec<crate::ranking::RankingSignal<T>> {
    #[cfg(feature = "postgres")]
    {
        postgres::finish_signals(signals)
    }
    #[cfg(not(feature = "postgres"))]
    {
        signals
    }
}

/// PostgreSQL text-search predicates with explicit configuration.
///
/// Unquoted values use `plainto_tsquery`; quoted values use `phraseto_tsquery`
/// (lexeme phrase search, including stop-word distances). Query operators in
/// user text are never interpreted as tsquery syntax. NULL targets yield SQL
/// NULL, also under NOT. Empty/stop-word-only queries match no rows.
#[cfg(feature = "postgres")]
pub mod postgres;

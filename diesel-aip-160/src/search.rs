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

/// PostgreSQL text-search predicates with explicit configuration.
///
/// Unquoted values use `plainto_tsquery`; quoted values use `phraseto_tsquery`
/// (lexeme phrase search, including stop-word distances). Query operators in
/// user text are never interpreted as tsquery syntax. NULL targets yield SQL
/// NULL, also under NOT. Empty/stop-word-only queries match no rows.
#[cfg(feature = "postgres")]
pub mod postgres {
    use anyhow::{Result, ensure};
    use diesel::expression::{
        AppearsOnTable, SelectableExpression, ValidGrouping, is_aggregate::No,
    };
    use diesel::pg::Pg;
    use diesel::query_builder::{AstPass, QueryFragment, QueryId};
    use diesel::sql_types::{Bool, Nullable, Text};
    use diesel::{Column, Expression, QueryResult};

    use crate::{diesel_helpers::Predicate, parser::Value};

    /// Diesel SQL type for a stored PostgreSQL search document. Applications may
    /// reference it in `table!` declarations; no Rust value codec is required
    /// when the document is used only as a search target.
    #[derive(diesel::sql_types::SqlType, diesel::query_builder::QueryId)]
    #[diesel(postgres_type(name = "tsvector"))]
    pub struct TsVector;

    mod sealed {
        pub trait TextTarget {}
        pub trait VectorTarget {}
    }
    impl sealed::TextTarget for Text {}
    impl sealed::TextTarget for Nullable<Text> {}
    impl sealed::VectorTarget for TsVector {}
    impl sealed::VectorTarget for Nullable<TsVector> {}

    /// SQL text types accepted by [`text`]. Integer and other nontext SQL
    /// targets are rejected at compile time, even if a model calls them String.
    ///
    /// ```compile_fail
    /// use diesel_aip_160::{search::postgres, parser::Value};
    /// diesel::table! { articles (id) { id -> Integer } }
    /// postgres::text(articles::id, "english", &Value { text: "x".into(), quoted: false });
    /// ```
    pub trait TextTarget: sealed::TextTarget {}
    impl<T: sealed::TextTarget> TextTarget for T {}
    /// SQL search-document types accepted by [`vector`]. A text source cannot
    /// accidentally be registered as a stored search document.
    ///
    /// ```compile_fail
    /// use diesel_aip_160::{search::postgres, parser::Value};
    /// diesel::table! { articles (id) { id -> Integer, title -> Text } }
    /// postgres::vector(articles::title, "english", &Value { text: "x".into(), quoted: false });
    /// ```
    pub trait VectorTarget: sealed::VectorTarget {}
    impl<T: sealed::VectorTarget> VectorTarget for T {}

    /// Search `to_tsvector(configuration, column)`. An expression index must use
    /// this exact expression/configuration; a separate stored-vector index will
    /// not accelerate this predicate. Configuration is a trusted static name,
    /// restricted to ASCII letters/digits/underscore/dot, rendered as a SQL
    /// constant so expression indexes can match it. User text is always bound.
    pub fn text<C>(
        column: C,
        configuration: &'static str,
        value: &Value,
    ) -> Result<Predicate<C::Table>>
    where
        C: Column + QueryFragment<Pg> + Send + 'static,
        C::SqlType: TextTarget,
    {
        predicate(column, configuration, value, false)
    }

    /// Search a registered stored `tsvector` column directly. Use the same
    /// configuration used to build the document. Its GIN index can serve this
    /// predicate. This helper is also useful inside a global `SearchAdapter`.
    pub fn vector<C>(
        column: C,
        configuration: &'static str,
        value: &Value,
    ) -> Result<Predicate<C::Table>>
    where
        C: Column + QueryFragment<Pg> + Send + 'static,
        C::SqlType: VectorTarget,
    {
        predicate(column, configuration, value, true)
    }

    fn predicate<C>(
        column: C,
        configuration: &'static str,
        value: &Value,
        vector: bool,
    ) -> Result<Predicate<C::Table>>
    where
        C: Column + QueryFragment<Pg> + Send + 'static,
    {
        ensure!(
            !configuration.is_empty()
                && configuration
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.'),
            "text-search configuration must be a static, optionally schema-qualified ASCII name"
        );
        Ok(Box::new(Search {
            column,
            configuration,
            query: value.text.clone(),
            phrase: value.quoted,
            vector,
        }))
    }

    struct Search<C> {
        column: C,
        configuration: &'static str,
        query: String,
        phrase: bool,
        vector: bool,
    }
    // As with native predicates, Bool is the boxed Diesel metadata. SQL still
    // propagates NULL; there is no COALESCE or change to three-valued logic.
    impl<C> Expression for Search<C> {
        type SqlType = Bool;
    }
    impl<C: Column> AppearsOnTable<C::Table> for Search<C> {}
    impl<C: Column> SelectableExpression<C::Table> for Search<C> {}
    impl<C> ValidGrouping<()> for Search<C> {
        type IsAggregate = No;
    }
    impl<C> QueryId for Search<C> {
        type QueryId = ();
        const HAS_STATIC_QUERY_ID: bool = false;
    }
    impl<C: QueryFragment<Pg>> QueryFragment<Pg> for Search<C> {
        fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
            out.unsafe_to_cache_prepared();
            out.push_sql("(");
            if !self.vector {
                out.push_sql("to_tsvector('");
                out.push_sql(self.configuration);
                out.push_sql("'::regconfig, ");
            }
            self.column.walk_ast(out.reborrow())?;
            if !self.vector {
                out.push_sql(")");
            }
            out.push_sql(if self.phrase {
                " @@ phraseto_tsquery('"
            } else {
                " @@ plainto_tsquery('"
            });
            out.push_sql(self.configuration);
            out.push_sql("'::regconfig, ");
            out.push_bind_param::<Text, _>(&self.query)?;
            out.push_sql("))");
            Ok(())
        }
    }
}

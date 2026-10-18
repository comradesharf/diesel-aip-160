use std::{any::TypeId, sync::Arc};

use anyhow::{Result, ensure};
use diesel::expression::{AppearsOnTable, SelectableExpression, ValidGrouping, is_aggregate::No};
use diesel::pg::Pg;
use diesel::query_builder::{AstPass, QueryFragment, QueryId};
use diesel::sql_types::{Bool, Nullable, Text};
use diesel::{Column, Expression, QueryResult};

use crate::{
    diesel_helpers::Predicate,
    ranking::RankingSignal,
    search::{SearchKind, SearchTerm},
};

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
/// use diesel_aip_160::{search::{postgres, SearchTerm}};
/// diesel::table! { articles (id) { id -> Integer } }
/// postgres::text(articles::id, "english", &SearchTerm::exact("x"));
/// ```
pub trait TextTarget: sealed::TextTarget {}
impl<T: sealed::TextTarget> TextTarget for T {}
/// SQL search-document types accepted by [`vector`]. A text source cannot
/// accidentally be registered as a stored search document.
///
/// ```compile_fail
/// use diesel_aip_160::{search::{postgres, SearchTerm}};
/// diesel::table! { articles (id) { id -> Integer, title -> Text } }
/// postgres::vector(articles::title, "english", &SearchTerm::exact("x"));
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
    value: &SearchTerm,
) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Pg> + Send + 'static,
    C::SqlType: TextTarget,
{
    predicate(column, configuration, value, Target::Text)
}

/// Search a registered stored `tsvector` column directly. Use the same
/// configuration used to build the document. Its GIN index can serve this
/// predicate. This helper is also useful inside a global `SearchAdapter`.
pub fn vector<C>(
    column: C,
    configuration: &'static str,
    value: &SearchTerm,
) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Pg> + Send + 'static,
    C::SqlType: VectorTarget,
{
    predicate(column, configuration, value, Target::Vector)
}

/// Match and rank the same text target/configuration/query. NULL scores
/// become zero; predicate NULL semantics remain unchanged.
pub fn text_query<C>(
    column: C,
    configuration: &'static str,
    value: &SearchTerm,
) -> Result<crate::ranking::CompiledQueryFilter<C::Table>>
where
    C: Column + Copy + QueryFragment<Pg> + Send + 'static,
    C::SqlType: TextTarget,
{
    ranked(column, configuration, value, Target::Text)
}

/// Match and rank the same stored search document.
pub fn vector_query<C>(
    column: C,
    configuration: &'static str,
    value: &SearchTerm,
) -> Result<crate::ranking::CompiledQueryFilter<C::Table>>
where
    C: Column + Copy + QueryFragment<Pg> + Send + 'static,
    C::SqlType: VectorTarget,
{
    ranked(column, configuration, value, Target::Vector)
}

fn ranked<C>(
    column: C,
    configuration: &'static str,
    value: &SearchTerm,
    target: Target,
) -> Result<crate::ranking::CompiledQueryFilter<C::Table>>
where
    C: Column + Copy + QueryFragment<Pg> + Send + 'static,
{
    use crate::ranking::Direction;
    let search = Search::new(column, configuration, value, target)?;
    let key = Arc::clone(&search.key);
    let mut signal = RankingSignal::new(
        "relevance",
        Rank(search.clone()),
        Direction::Descending,
        "postgres.full_text",
    );
    signal.provenance = Some(key);
    Ok(crate::compiler::QueryFilter {
        predicate: Box::new(search),
        signals: vec![signal],
    })
}

struct Rank<C>(Search<C>);
impl<C> Expression for Rank<C> {
    type SqlType = diesel::sql_types::Float;
}
impl<C: Column> AppearsOnTable<C::Table> for Rank<C> {}
impl<C: Column> SelectableExpression<C::Table> for Rank<C> {}
impl<C> ValidGrouping<()> for Rank<C> {
    type IsAggregate = No;
}
impl<C> QueryId for Rank<C> {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}
impl<C: QueryFragment<Pg>> QueryFragment<Pg> for Rank<C> {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.unsafe_to_cache_prepared();
        out.push_sql("COALESCE(ts_rank_cd(");
        self.0.target(out.reborrow())?;
        out.push_sql(", ");
        self.0.tsquery(out.reborrow())?;
        out.push_sql("), 0.0::real)");
        Ok(())
    }
}

fn predicate<C>(
    column: C,
    configuration: &'static str,
    value: &SearchTerm,
    target: Target,
) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Pg> + Send + 'static,
{
    Ok(Box::new(Search::new(column, configuration, value, target)?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Target {
    Text,
    Vector,
}

/// Validated search identity shared by matching, ranking, and deduplication.
#[derive(Debug, PartialEq, Eq, Hash)]
struct SearchKey {
    column: TypeId,
    configuration: &'static str,
    query: String,
    kind: SearchKind,
    target: Target,
}

#[derive(Clone)]
struct Search<C> {
    column: C,
    key: Arc<SearchKey>,
}
impl<C: 'static> Search<C> {
    fn new(
        column: C,
        configuration: &'static str,
        value: &SearchTerm,
        target: Target,
    ) -> Result<Self> {
        ensure!(
            !configuration.is_empty()
                && configuration
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.'),
            "text-search configuration must be a static, optionally schema-qualified ASCII name"
        );
        if value.kind == SearchKind::Prefix {
            SearchTerm::prefix(&value.text)?;
        }
        Ok(Self {
            column,
            key: Arc::new(SearchKey {
                column: TypeId::of::<C>(),
                configuration,
                query: value.text.clone(),
                kind: value.kind,
                target,
            }),
        })
    }
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
impl<C: QueryFragment<Pg>> Search<C> {
    fn target<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        if self.key.target == Target::Text {
            out.push_sql("to_tsvector('");
            out.push_sql(self.key.configuration);
            out.push_sql("'::regconfig, ");
        }
        self.column.walk_ast(out.reborrow())?;
        if self.key.target == Target::Text {
            out.push_sql(")");
        }
        Ok(())
    }
    fn tsquery<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        if self.key.kind == SearchKind::Prefix {
            // Normalize once using the document configuration. Serialize each
            // lexeme using tsquery (not SQL string) escaping, then cast directly
            // to tsquery so dictionaries cannot normalize lexemes a second time.
            // The scalar subquery is independent of the row and index eligible.
            out.push_sql("(SELECT COALESCE(string_agg(chr(39) || replace(replace(lexeme, chr(92), chr(92) || chr(92)), chr(39), chr(39) || chr(39)) || chr(39) || ':*', ' & ' ORDER BY lexeme), '')::tsquery FROM unnest(tsvector_to_array(to_tsvector('");
            out.push_sql(self.key.configuration);
            out.push_sql("'::regconfig, ");
            out.push_bind_param::<Text, _>(&self.key.query)?;
            out.push_sql("))) AS aip160_prefix(lexeme))");
        } else {
            out.push_sql(if self.key.kind == SearchKind::Phrase {
                "phraseto_tsquery('"
            } else {
                "plainto_tsquery('"
            });
            out.push_sql(self.key.configuration);
            out.push_sql("'::regconfig, ");
            out.push_bind_param::<Text, _>(&self.key.query)?;
            out.push_sql(")");
        }
        Ok(())
    }
}
impl<C: QueryFragment<Pg>> QueryFragment<Pg> for Search<C> {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.unsafe_to_cache_prepared();
        out.push_sql("(");
        self.target(out.reborrow())?;
        out.push_sql(" @@ ");
        self.tsquery(out.reborrow())?;
        out.push_sql(")");
        Ok(())
    }
}

struct BestTextScore(Vec<crate::ranking::ErasedExpression>);
impl QueryFragment<Pg> for BestTextScore {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.push_sql("GREATEST(");
        for (i, e) in self.0.iter().enumerate() {
            if i != 0 {
                out.push_sql(", ");
            }
            e.walk_ast(out.reborrow())?;
        }
        out.push_sql(")");
        Ok(())
    }
}
/// Internal text policy: deduplicate identical target/configuration/query
/// contributions, then use the strongest built-in text relevance. Maximum is
/// invariant under equivalent queries even when dictionaries normalize different
/// literals to the same lexemes. Other metrics are never combined.
#[doc(hidden)]
pub(super) fn finish_signals<T>(signals: Vec<RankingSignal<T>>) -> Vec<RankingSignal<T>> {
    let mut result = Vec::new();
    let mut keys = std::collections::HashSet::new();
    let mut text: Option<RankingSignal<T>> = None;
    let mut expressions = Vec::new();
    for signal in signals {
        let key = signal
            .provenance
            .as_ref()
            .and_then(|data| Arc::clone(data).downcast::<SearchKey>().ok());
        if let Some(key) = key {
            if keys.insert(key) {
                if text.is_none() {
                    text = Some(signal);
                } else {
                    expressions.push(signal.expression);
                }
            }
        } else {
            result.push(signal);
        }
    }
    if let Some(mut signal) = text {
        if !expressions.is_empty() {
            expressions.insert(0, signal.expression);
            signal.expression = Box::new(BestTextScore(expressions));
        }
        // A finalized composite is not a raw leaf contribution. Keeping a leaf
        // key here could let a later compiler mistakenly deduplicate the whole
        // composite against just its first term.
        signal.provenance = None;
        result.push(signal);
    }
    result
}

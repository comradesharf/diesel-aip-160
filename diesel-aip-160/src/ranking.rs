//! Independent typed ranking signals and explicit ordering policies.
//!
//! No ordering is installed by compilation. Policies select named signals,
//! retaining their SQL types and preferred directions. Generic composition
//! collects positive leaves even in unmatched branches; odd NOT polarity drops
//! signals and double negation restores them. Adapters define metric semantics
//! and NULL handling. Built-in PostgreSQL relevance takes the maximum positive
//! leaf score (zero for NULL), deduplicates identical contributions, and never
//! rewards negative or repeated equivalent terms. Other metrics are independent.
//!
//! ```
//! use diesel::prelude::*;
//! use diesel_aip_160::{compiler::QueryFilter, ranking::{Direction, RankingSignal, CompiledQueryFilter}};
//! diesel::table! { places (id) { id -> Integer, label -> Text } }
//! diesel::define_sql_function! { fn abs(value: diesel::sql_types::Integer) -> diesel::sql_types::Integer; }
//! let mut compiled: CompiledQueryFilter<places::table> = QueryFilter {
//!     predicate: Box::new(places::id.ge(0)),
//!     signals: vec![
//!         RankingSignal::new("distance", abs(places::id - 7), Direction::Ascending, "demo.distance"),
//!         RankingSignal::new("label", places::label, Direction::Descending, "demo.label"),
//!     ],
//! };
//! let order = compiled.ordering(&["distance", "label"])?;
//! let query = places::table.filter(compiled.predicate)
//!     .order(order).then_order_by(places::id).limit(20).offset(40);
//! # let _ = query;
//! # Ok::<(), diesel_aip_160::anyhow::Error>(())
//! ```
#[cfg(feature = "postgres")]
use std::{any::Any, sync::Arc};
use std::{any::TypeId, marker::PhantomData};

use crate::{
    compiler::QueryFilter,
    diesel_helpers::{Backend, Predicate},
};
use anyhow::{Result, ensure};
use diesel::{
    Expression, QueryResult,
    expression::{AppearsOnTable, SelectableExpression, ValidGrouping, is_aggregate::No},
    query_builder::{AstPass, QueryFragment, QueryId},
};

/// A Diesel predicate and the independent signals produced by query compilation.
pub type CompiledQueryFilter<T> = QueryFilter<Predicate<T>, RankingSignal<T>>;

/// Preferred ordering for a metric. Applications may override it via a typed expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ascending,
    Descending,
}

/// A named metric. SQL types remain distinct; only the built-in text-search
/// implementation combines its own relevance contributions.
pub struct RankingSignal<T> {
    /// Application-visible metric name. Repeated names require disambiguation.
    pub name: String,
    /// Default direction used by explicit ordering policies.
    pub direction: Direction,
    /// Operation that produced this metric, such as `postgres.full_text`.
    pub operation: String,
    sql_type: &'static str,
    type_id: TypeId,
    pub(crate) expression: ErasedExpression,
    table: PhantomData<fn() -> T>,
    // Only PostgreSQL currently emits built-in contributions. Their opaque
    // composition data is interpreted by that operation, never the generic layer.
    #[cfg(feature = "postgres")]
    pub(crate) provenance: Option<Arc<dyn Any + Send + Sync>>,
}
// SQL structure is erased only after Diesel has checked the table, SQL type,
// and aggregate status in RankingSignal::new. Type recovery never casts SQL.
pub(crate) type ErasedExpression = Box<dyn QueryFragment<Backend> + Send>;

impl<T> RankingSignal<T> {
    /// Rust name of the expression's Diesel SQL type (diagnostic metadata).
    pub fn sql_type(&self) -> &'static str {
        self.sql_type
    }

    /// Accept a typed, nonaggregate Diesel expression. Bind values as usual.
    pub fn new<E>(
        name: impl Into<String>,
        expression: E,
        direction: Direction,
        operation: impl Into<String>,
    ) -> Self
    where
        E: diesel::BoxableExpression<T, Backend> + 'static,
        E::SqlType: 'static,
    {
        Self {
            name: name.into(),
            direction,
            operation: operation.into(),
            sql_type: std::any::type_name::<E::SqlType>(),
            type_id: TypeId::of::<E::SqlType>(),
            expression: Box::new(expression),
            table: PhantomData,
            #[cfg(feature = "postgres")]
            provenance: None,
        }
    }
    /// Recover a typed expression for application-defined ordering or selection.
    /// A mismatched SQL type returns an error instead of silently casting SQL.
    pub fn into_expression<ST: diesel::expression::TypedExpressionType + 'static>(
        self,
    ) -> Result<RankingExpression<T, ST>> {
        ensure!(
            self.type_id == TypeId::of::<ST>(),
            "signal `{}` has SQL type {}, requested {}",
            self.name,
            self.sql_type,
            std::any::type_name::<ST>()
        );
        Ok(RankingExpression {
            expression: self.expression,
            marker: PhantomData,
        })
    }
    /// Consume this signal and use its preferred direction in ORDER BY.
    pub fn into_ordering(self) -> Ordering<T> {
        Ordering {
            terms: vec![(self.expression, self.direction)],
            table: PhantomData,
        }
    }
}

/// Typed Diesel expression, usable with `.asc()`, `.desc()`, or `.select()`.
pub struct RankingExpression<T, ST> {
    expression: ErasedExpression,
    marker: PhantomData<fn() -> (T, ST)>,
}
impl<T, ST: diesel::expression::TypedExpressionType> Expression for RankingExpression<T, ST> {
    type SqlType = ST;
}
impl<T, ST> AppearsOnTable<T> for RankingExpression<T, ST> where Self: Expression {}
impl<T, ST> SelectableExpression<T> for RankingExpression<T, ST> where Self: Expression {}
impl<T, ST> ValidGrouping<()> for RankingExpression<T, ST> {
    type IsAggregate = No;
}
impl<T, ST> QueryId for RankingExpression<T, ST> {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}
impl<T, ST> QueryFragment<Backend> for RankingExpression<T, ST> {
    fn walk_ast<'b>(&'b self, out: AstPass<'_, 'b, Backend>) -> QueryResult<()> {
        self.expression.walk_ast(out)
    }
}

/// One or more signals in priority order, each using its preferred direction.
/// Applications append tie-breakers with `.then_order_by(...)` before pagination.
pub struct Ordering<T> {
    terms: Vec<(ErasedExpression, Direction)>,
    table: PhantomData<fn() -> T>,
}
impl<T> Expression for Ordering<T> {
    type SqlType = diesel::expression::expression_types::NotSelectable;
}
impl<T> AppearsOnTable<T> for Ordering<T> {}
impl<T> ValidGrouping<()> for Ordering<T> {
    type IsAggregate = No;
}
impl<T> QueryId for Ordering<T> {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}
impl<T> QueryFragment<Backend> for Ordering<T> {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Backend>) -> QueryResult<()> {
        for (i, (expression, direction)) in self.terms.iter().enumerate() {
            if i != 0 {
                out.push_sql(", ");
            }
            expression.walk_ast(out.reborrow())?;
            out.push_sql(match direction {
                Direction::Ascending => " ASC",
                Direction::Descending => " DESC",
            });
        }
        Ok(())
    }
}
impl<T> CompiledQueryFilter<T> {
    /// Remove a uniquely named signal. Missing or ambiguous names are errors.
    pub fn take_signal(&mut self, name: &str) -> Result<RankingSignal<T>> {
        let position = self.signal_position(name)?;
        Ok(self.signals.remove(position))
    }

    fn signal_position(&self, name: &str) -> Result<usize> {
        let mut matches = self
            .signals
            .iter()
            .enumerate()
            .filter(|(_, signal)| signal.name == name);
        let (position, _) = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing signal `{name}`"))?;
        ensure!(matches.next().is_none(), "ambiguous signal `{name}`");
        Ok(position)
    }

    /// Select one signal or several in priority order. No implicit ordering is
    /// installed. Validate the whole policy before consuming any signals.
    pub fn ordering(&mut self, names: &[&str]) -> Result<Ordering<T>> {
        ensure!(!names.is_empty(), "ordering requires at least one signal");
        for (i, name) in names.iter().enumerate() {
            ensure!(
                !names[..i].contains(name),
                "duplicate ordering signal `{name}`"
            );
            self.signal_position(name)?;
        }
        let mut terms = Vec::with_capacity(names.len());
        for name in names {
            let signal = self.take_signal(name)?;
            terms.push((signal.expression, signal.direction));
        }
        Ok(Ordering {
            terms,
            table: PhantomData,
        })
    }
}

//! Opt-in full-text search. Built-in support currently requires PostgreSQL.
//! Applications can implement `diesel_helpers::SearchAdapter` on every
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
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
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
/// Exact terms use `plainto_tsquery`; phrases use `phraseto_tsquery`
/// (lexeme phrase search, including stop-word distances). Query operators in
/// user text are never interpreted as tsquery syntax. Prefix terms normalize
/// text with the configuration and build an escaped prefix tsquery. NULL targets yield SQL
/// NULL, also under NOT. Empty/stop-word-only queries match no rows.
#[cfg(feature = "postgres")]
pub mod postgres;

/// Search semantics, independent of filter quoting or raw tsquery syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchKind {
    /// PostgreSQL: all normalized lexemes must match whole tokens.
    /// Other adapters (including default substring search) define their own policy.
    Exact,
    /// Ordered lexemes, including stop-word distances.
    Phrase,
    /// All normalized lexemes must match token prefixes.
    Prefix,
}

/// Literal user text with explicit search semantics. Constructors never parse
/// filter syntax or tsquery operators. Normalization belongs to the adapter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SearchTerm {
    pub text: String,
    pub kind: SearchKind,
}
impl SearchTerm {
    /// Build a global typed expression without interpreting text as filter syntax.
    pub fn global(self) -> crate::parser::Expression {
        crate::parser::Expression::Search {
            field: None,
            term: self,
        }
    }
    /// Build a field search; compilation validates field registration/capability.
    pub fn field(self, field: impl Into<String>) -> crate::parser::Expression {
        crate::parser::Expression::Search {
            field: Some(field.into()),
            term: self,
        }
    }
    pub fn exact(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SearchKind::Exact,
        }
    }
    pub fn phrase(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: SearchKind::Phrase,
        }
    }
    /// A nonempty prefix containing at least one Unicode letter or number.
    /// Punctuation is literal text, processed by the backend's tokenizer.
    pub fn prefix(text: impl Into<String>) -> anyhow::Result<Self> {
        let text = text.into();
        anyhow::ensure!(
            text.chars().any(char::is_alphanumeric),
            "prefix search requires a nonempty prefix containing a letter or number"
        );
        Ok(Self {
            text,
            kind: SearchKind::Prefix,
        })
    }
    /// Interpret the trailing-wildcard dialect extension only in search context.
    /// Quoted text never enables operators. Bare `*` is not an empty prefix.
    pub fn from_literal(value: &crate::parser::Value) -> anyhow::Result<Self> {
        if value.quoted {
            return Ok(Self::phrase(&value.text));
        }
        if value.text.contains('*') {
            anyhow::ensure!(
                value.text.ends_with('*') && value.text.matches('*').count() == 1,
                "search supports only one trailing wildcard; quote literal asterisks"
            );
            return Self::prefix(&value.text[..value.text.len() - 1]);
        }
        Ok(Self::exact(&value.text))
    }
}

/// Build global autocomplete from arbitrary text without parsing filter syntax.
/// Unicode whitespace separates chunks; punctuation-only chunks are discarded.
/// The last retained chunk is a prefix, preceding chunks are exact searches,
/// joined with AND. Trailing whitespace does not finalize the last chunk.
/// Case, Unicode, and punctuation within chunks are preserved for PostgreSQL's
/// configuration-aware tokenizer; there is no client-side stemming or folding.
/// Empty/punctuation-only input returns None. Input limits match the parser.
pub fn autocomplete(input: &str) -> anyhow::Result<Option<crate::parser::Expression>> {
    autocomplete_for(None, input)
}

/// Like [`autocomplete`], restricted to a registered searchable field. The field
/// is an identifier, not filter syntax, and is validated during compilation.
pub fn autocomplete_field(
    field: &str,
    input: &str,
) -> anyhow::Result<Option<crate::parser::Expression>> {
    autocomplete_for(Some(field), input)
}
fn autocomplete_for(
    field: Option<&str>,
    input: &str,
) -> anyhow::Result<Option<crate::parser::Expression>> {
    use crate::parser::{Expression, MAX_FILTER_BYTES, MAX_TOKENS};
    anyhow::ensure!(
        input.len() <= MAX_FILTER_BYTES,
        "autocomplete input exceeds {MAX_FILTER_BYTES} bytes"
    );
    anyhow::ensure!(
        !input.chars().any(|c| c.is_control() && !c.is_whitespace()),
        "autocomplete input contains a control character"
    );
    let mut chunks: Vec<_> = input
        .split_whitespace()
        .filter(|s| s.chars().any(char::is_alphanumeric))
        .collect();
    anyhow::ensure!(
        chunks.len() <= MAX_TOKENS,
        "autocomplete input exceeds {MAX_TOKENS} terms"
    );
    let Some(last) = chunks.pop() else {
        return Ok(None);
    };
    let mut terms: Vec<_> = chunks.into_iter().map(SearchTerm::exact).collect();
    terms.push(SearchTerm::prefix(last)?);
    let mut expressions: Vec<_> = terms
        .into_iter()
        .map(|term| Expression::Search {
            field: field.map(str::to_owned),
            term,
        })
        .collect();
    Ok(Some(if expressions.len() == 1 {
        expressions.remove(0)
    } else {
        Expression::And(expressions)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Expression, Value};

    #[test]
    fn search_syntax_preserves_literal_protection_and_rejects_bad_wildcards() {
        for (text, quoted, expected) in [
            ("sen*", false, SearchTerm::prefix("sen").unwrap()),
            ("sentral", false, SearchTerm::exact("sentral")),
            ("sen*", true, SearchTerm::phrase("sen*")),
            ("Kuala Lumpur", true, SearchTerm::phrase("Kuala Lumpur")),
        ] {
            assert_eq!(
                SearchTerm::from_literal(&Value {
                    text: text.into(),
                    quoted
                })
                .unwrap(),
                expected
            );
        }
        for text in ["*", "**", "*sen", "s*n", "sen**", "!!!*"] {
            assert!(
                SearchTerm::from_literal(&Value {
                    text: text.into(),
                    quoted: false
                })
                .is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn autocomplete_is_a_typed_conjunction_and_never_parses_operators() {
        assert_eq!(autocomplete(" \t\n !!! ").unwrap(), None);
        for input in [
            "s",
            "sen",
            "T7",
            "吉隆坡",
            "ÉCO",
            "sen*",
            "x'|!y:*",
            r"foo\bar",
        ] {
            assert_eq!(
                autocomplete(input).unwrap(),
                Some(Expression::Search {
                    field: None,
                    term: SearchTerm::prefix(input).unwrap(),
                })
            );
        }
        assert_eq!(
            autocomplete_field("long_name", "  kuala \t !!! sen  ").unwrap(),
            Some(Expression::And(vec![
                Expression::Search {
                    field: Some("long_name".into()),
                    term: SearchTerm::exact("kuala")
                },
                Expression::Search {
                    field: Some("long_name".into()),
                    term: SearchTerm::prefix("sen").unwrap()
                },
            ]))
        );
        let Some(Expression::And(terms)) = autocomplete("title:sentral OR NOT (x)").unwrap() else {
            panic!()
        };
        assert_eq!(terms.len(), 4); // OR and NOT are ordinary exact text.
        assert!(autocomplete("\0").is_err());
        assert!(autocomplete(&"x".repeat(crate::parser::MAX_FILTER_BYTES + 1)).is_err());
        assert!(autocomplete(&"x ".repeat(crate::parser::MAX_TOKENS + 1)).is_err());
    }
}

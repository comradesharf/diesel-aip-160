//! AIP-160 syntax, independent of field types and the database backend.
//!
//! Follows https://google.aip.dev/assets/misc/ebnf-filtering.txt.
//! OR binds more tightly than both explicit AND and whitespace conjunctions.

use anyhow::{Result, bail, ensure};

pub const MAX_FILTER_BYTES: usize = 16_384;
pub const MAX_TOKENS: usize = 512;
pub const MAX_NESTING: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expression {
    And(Vec<Expression>),
    Or(Vec<Expression>),
    Not(Box<Expression>),
    Restriction {
        left: Comparable,
        operator: Comparator,
        right: Argument,
    },
    Global(Comparable),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Has,
}

/// Preserve spelling and quoting: `001`, `true` and `null` are interpreted by
/// the field's type, rather than prematurely converted to numbers or booleans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub text: String,
    pub quoted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Comparable {
    Member(Vec<Value>),
    Function {
        name: String,
        arguments: Vec<Argument>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    Comparable(Comparable),
    Composite(Box<Expression>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Value(Value),
    Comparator(Comparator),
    LeftParen,
    RightParen,
    Dot,
    Comma,
    Minus,
}

#[derive(Debug)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

/// Empty/whitespace-only filters impose no restriction. Errors include a byte
/// offset. Limits bound parser recursion, AST destruction and generated SQL.
pub fn parse_filter(source: &str) -> Result<Option<Expression>> {
    ensure!(
        source.len() <= MAX_FILTER_BYTES,
        "filter exceeds {MAX_FILTER_BYTES} bytes"
    );
    let tokens = tokenize(source)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let mut parser = Parser {
        tokens,
        position: 0,
        source_len: source.len(),
    };
    let expression = parser.expression(0)?;
    ensure!(
        parser.peek().is_none(),
        "unexpected token at byte {}",
        parser.offset()
    );
    Ok(Some(expression))
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut chars = source.char_indices().peekable();
    let mut tokens = Vec::new();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        ensure!(
            !ch.is_control(),
            "unexpected control character at byte {start}"
        );
        ensure!(
            tokens.len() < MAX_TOKENS,
            "filter exceeds {MAX_TOKENS} tokens"
        );
        let kind = match ch {
            '(' => Kind::LeftParen,
            ')' => Kind::RightParen,
            '.' => Kind::Dot,
            ',' => Kind::Comma,
            '-' => Kind::Minus,
            ':' => Kind::Comparator(Comparator::Has),
            '=' => Kind::Comparator(Comparator::Equal),
            '<' | '>' | '!' => {
                let equal = chars.peek().is_some_and(|(_, next)| *next == '=');
                if equal {
                    chars.next();
                }
                Kind::Comparator(match (ch, equal) {
                    ('<', false) => Comparator::LessThan,
                    ('<', true) => Comparator::LessThanOrEqual,
                    ('>', false) => Comparator::GreaterThan,
                    ('>', true) => Comparator::GreaterThanOrEqual,
                    ('!', true) => Comparator::NotEqual,
                    _ => bail!("expected != at byte {start}; use NOT for negation"),
                })
            }
            '\'' | '"' => {
                let mut text = String::new();
                let mut closed = false;
                while let Some((offset, next)) = chars.next() {
                    if next == ch {
                        closed = true;
                        break;
                    }
                    if next == '\\' {
                        let (_, escaped) = chars
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("unfinished escape at byte {offset}"))?;
                        text.push(match escaped {
                            '\\' | '\'' | '"' => escaped,
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            _ => bail!("unsupported escape at byte {offset}"),
                        });
                    } else {
                        ensure!(
                            !next.is_control(),
                            "unescaped control character at byte {offset}"
                        );
                        text.push(next);
                    }
                }
                ensure!(closed, "unterminated string at byte {start}");
                Kind::Value(Value { text, quoted: true })
            }
            '&' | '|' | '[' | ']' | '\\' => bail!("unexpected character `{ch}` at byte {start}"),
            _ => {
                while chars.peek().is_some_and(|(_, next)| {
                    !next.is_whitespace()
                        && !next.is_control()
                        && !"().,:=<>!\"'&|[]\\".contains(*next)
                }) {
                    chars.next();
                }
                let end = chars.peek().map_or(source.len(), |(offset, _)| *offset);
                Kind::Value(Value {
                    text: source[start..end].to_owned(),
                    quoted: false,
                })
            }
        };
        let end = chars.peek().map_or(source.len(), |(offset, _)| *offset);
        tokens.push(Token { kind, start, end });
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    source_len: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.position).map(|token| &token.kind)
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.position)
            .map_or(self.source_len, |token| token.start)
    }

    fn keyword(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Kind::Value(value)) if !value.quoted && value.text == name)
    }

    fn whitespace_before(&self) -> bool {
        self.position > 0
            && self
                .tokens
                .get(self.position)
                .is_some_and(|token| token.start > self.tokens[self.position - 1].end)
    }

    fn take(&mut self, kind: &Kind) -> bool {
        if self.peek() == Some(kind) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn logical_operator(&mut self) -> Result<()> {
        ensure!(
            self.whitespace_before(),
            "expected whitespace before operator at byte {}",
            self.offset()
        );
        self.position += 1;
        ensure!(
            self.whitespace_before(),
            "expected whitespace after operator at byte {}",
            self.offset()
        );
        Ok(())
    }

    fn expression(&mut self, depth: usize) -> Result<Expression> {
        ensure!(
            depth <= MAX_NESTING,
            "filter exceeds {MAX_NESTING} levels of nesting"
        );
        let mut terms = vec![self.factor(depth)?];
        loop {
            if self.keyword("AND") {
                self.logical_operator()?;
            } else if self.peek().is_none()
                || matches!(self.peek(), Some(Kind::RightParen | Kind::Comma))
            {
                break;
            } else {
                ensure!(
                    self.whitespace_before(),
                    "expected whitespace or operator at byte {}",
                    self.offset()
                );
            }
            terms.push(self.factor(depth)?);
        }
        Ok(if terms.len() == 1 {
            terms.remove(0)
        } else {
            Expression::And(terms)
        })
    }

    fn factor(&mut self, depth: usize) -> Result<Expression> {
        let mut terms = vec![self.term(depth)?];
        while self.keyword("OR") {
            self.logical_operator()?;
            terms.push(self.term(depth)?);
        }
        Ok(if terms.len() == 1 {
            terms.remove(0)
        } else {
            Expression::Or(terms)
        })
    }

    fn term(&mut self, depth: usize) -> Result<Expression> {
        let negate = if self.keyword("NOT")
            && (!self.is_function_start()
                || self
                    .tokens
                    .get(self.position + 1)
                    .is_some_and(|next| next.start > self.tokens[self.position].end))
        {
            self.position += 1;
            ensure!(
                self.whitespace_before(),
                "expected whitespace after NOT at byte {}",
                self.offset()
            );
            true
        } else {
            self.take(&Kind::Minus)
        };
        let expression = if self.take(&Kind::LeftParen) {
            self.composite(depth + 1)?
        } else {
            let left = self.comparable(depth, false)?;
            if let Some(Kind::Comparator(operator)) = self.peek() {
                let operator = *operator;
                self.position += 1;
                let right = self.argument(depth)?;
                Expression::Restriction {
                    left,
                    operator,
                    right,
                }
            } else {
                Expression::Global(left)
            }
        };
        Ok(if negate {
            Expression::Not(Box::new(expression))
        } else {
            expression
        })
    }

    // Keywords may be function names, but otherwise delimit logical terms.
    fn is_function_start(&self) -> bool {
        let mut position = self.position + 1;
        while matches!(self.tokens.get(position).map(|t| &t.kind), Some(Kind::Dot)) {
            position += 2;
        }
        matches!(
            self.tokens.get(position).map(|t| &t.kind),
            Some(Kind::LeftParen)
        )
    }

    fn composite(&mut self, depth: usize) -> Result<Expression> {
        let expression = self.expression(depth)?;
        ensure!(
            self.take(&Kind::RightParen),
            "expected ')' at byte {}",
            self.offset()
        );
        Ok(expression)
    }

    fn argument(&mut self, depth: usize) -> Result<Argument> {
        if self.take(&Kind::LeftParen) {
            Ok(Argument::Composite(Box::new(self.composite(depth + 1)?)))
        } else {
            Ok(Argument::Comparable(self.comparable(depth, true)?))
        }
    }

    fn comparable(&mut self, depth: usize, allow_negative: bool) -> Result<Comparable> {
        ensure!(
            depth <= MAX_NESTING,
            "filter exceeds {MAX_NESTING} levels of nesting"
        );
        let keyword = self.keyword("AND") || self.keyword("OR") || self.keyword("NOT");
        ensure!(
            !keyword || self.is_function_start(),
            "expected value at byte {}",
            self.offset()
        );
        // Negative numeric/duration literals on a comparison's right side.
        let negative = allow_negative && self.take(&Kind::Minus);
        let mut first = self.value()?;
        if negative {
            ensure!(
                !first.quoted
                    && first.text.starts_with(|c: char| c.is_ascii_digit())
                    && self.tokens[self.position - 1].start == self.tokens[self.position - 2].end,
                "expected number after '-' at byte {}",
                self.offset()
            );
            first.text.insert(0, '-');
        }
        let mut parts = vec![first];
        while self.take(&Kind::Dot) {
            parts.push(self.value()?);
        }
        if self.take(&Kind::LeftParen) {
            ensure!(
                depth < MAX_NESTING,
                "filter exceeds {MAX_NESTING} levels of nesting"
            );
            ensure!(
                parts
                    .iter()
                    .all(|part| !part.quoted && valid_name(&part.text)),
                "invalid function name at byte {}",
                self.offset()
            );
            let name = parts
                .into_iter()
                .map(|part| part.text)
                .collect::<Vec<_>>()
                .join(".");
            let mut arguments = Vec::new();
            if !self.take(&Kind::RightParen) {
                loop {
                    arguments.push(self.argument(depth + 1)?);
                    if self.take(&Kind::RightParen) {
                        break;
                    }
                    ensure!(
                        self.take(&Kind::Comma),
                        "expected ',' or ')' at byte {}",
                        self.offset()
                    );
                }
            }
            Ok(Comparable::Function { name, arguments })
        } else {
            Ok(Comparable::Member(parts))
        }
    }

    fn value(&mut self) -> Result<Value> {
        let Some(Kind::Value(value)) = self.peek() else {
            bail!("expected value at byte {}", self.offset())
        };
        let value = value.clone();
        self.position += 1;
        Ok(value)
    }
}

pub(super) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Expression {
        parse_filter(source).unwrap().unwrap()
    }

    #[test]
    fn or_binds_more_tightly_than_explicit_and_implicit_and() {
        assert_eq!(parse("a AND b OR c"), parse("a AND (b OR c)"));
        assert_eq!(parse("a b OR c"), parse("a AND (b OR c)"));
        assert_eq!(parse("a OR b c OR d"), parse("(a OR b) AND (c OR d)"));
        assert_ne!(parse("a AND b OR c"), parse("(a AND b) OR c"));
    }

    #[test]
    fn supports_both_negations_and_nested_groups() {
        assert_eq!(parse("NOT (a OR b)"), parse("-(a OR b)"));
        assert_eq!(parse("NOT a"), parse("-a"));
        assert_eq!(parse("NOT NOT(a)"), parse("-NOT(a)"));
        assert!(matches!(
            parse("NOT(a)"),
            Expression::Global(Comparable::Function { .. })
        ));
    }

    #[test]
    fn preserves_literals_until_field_types_are_known() {
        for (source, text, quoted) in [
            ("field=001", "001", false),
            ("field=true", "true", false),
            ("field=null", "null", false),
            ("field='null'", "null", true),
            ("field=-2147483648", "-2147483648", false),
            ("field=9007199254740993", "9007199254740993", false),
            ("field=2e+9", "2e+9", false),
            ("field=1s", "1s", false),
            ("field=\"O'Reilly\"", "O'Reilly", true),
            (r#"field="a\"b\\c\n\t""#, "a\"b\\c\n\t", true),
            ("field='吉隆坡'", "吉隆坡", true),
        ] {
            let Expression::Restriction {
                right: Argument::Comparable(Comparable::Member(parts)),
                ..
            } = parse(source)
            else {
                panic!("{source}")
            };
            assert_eq!(
                parts,
                vec![Value {
                    text: text.to_owned(),
                    quoted
                }],
                "{source}"
            );
        }
    }

    #[test]
    fn parses_traversal_functions_and_composite_arguments() {
        let expression = parse("user.name = (Alice OR Bob)");
        let Expression::Restriction {
            left: Comparable::Member(parts),
            right: Argument::Composite(_),
            ..
        } = expression
        else {
            panic!("restriction expected")
        };
        assert_eq!(
            parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
            ["user", "name"]
        );
        let Expression::Global(Comparable::Function { name, arguments }) =
            parse("math.check(user.name, helper(), (a OR b))")
        else {
            panic!("function expected")
        };
        assert_eq!(name, "math.check");
        assert_eq!(arguments.len(), 3);
        for source in [
            "map.AND:*",
            "name = com.google",
            "amount >= -2.997e9",
            "duration=1.2s",
        ] {
            parse(source);
        }
    }

    #[test]
    fn rejects_malformed_or_partially_consumed_input() {
        for source in [
            "()",
            "(",
            "a)",
            "(a",
            "a AND",
            "a OR",
            "NOT",
            "NOT(a OR b)",
            "a OR(b)",
            "(a)AND b",
            "a AND(b)",
            "a(b)c",
            "a=",
            "a==b",
            "a===b",
            "a ! b",
            "a && b",
            "a || b",
            "a=b=c",
            "a='unterminated",
            "a='bad\\q'",
            "a='trailing\\",
            "a='line\nbreak'",
            "a..b=c",
            "a.=b",
            "a[0]=b",
            "a,b",
            "f(a,)",
            "f(,a)",
            "f(a b)",
            "f((a)",
            "--a",
            "--30",
            "a = - 30",
        ] {
            assert!(
                parse_filter(source).is_err(),
                "unexpectedly accepted: {source}"
            );
        }
    }

    #[test]
    fn rejects_unquoted_control_characters_with_byte_offsets() {
        for (source, offset) in [("\0", 0), ("name=a\0b", 6), ("name=吉\u{7f}", 8)] {
            let error = parse_filter(source).unwrap_err().to_string();
            assert!(error.contains(&format!("byte {offset}")), "{error}");
        }
    }

    #[test]
    fn recognizes_empty_filters_and_enforces_resource_limits() {
        assert_eq!(parse_filter(" \n\t ").unwrap(), None);
        assert!(parse_filter(&"a".repeat(MAX_FILTER_BYTES + 1)).is_err());
        assert!(parse_filter(&"a ".repeat(MAX_TOKENS + 1)).is_err());
        let nested = format!(
            "{}a{}",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert!(parse_filter(&nested).is_err());
        let calls = format!(
            "{}a{}",
            "f(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert!(parse_filter(&calls).is_err());
        let empty_calls = format!(
            "{}{}",
            "f(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert!(parse_filter(&empty_calls).is_err());
        let at_limit = format!("{}a{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
        assert_eq!(parse(&at_limit), parse("a"));
    }
}

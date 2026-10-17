//! Parse and validate literals before binding them to SQL.

use super::{Bound, Kind, Type};
use crate::parser::Value;
#[cfg(not(feature = "postgres"))]
use anyhow::bail;
use anyhow::{Result, ensure};

pub(super) fn parse(ty: &Type, value: &Value) -> Result<Bound> {
    let text = &value.text;
    ensure!(
        !ty.unsigned || !text.starts_with('-'),
        "unsigned value cannot be negative"
    );
    match &ty.kind {
        Kind::Signed(bits) => {
            ensure!(
                !value.quoted,
                "requires an unquoted signed {bits}-bit integer"
            );
            let n: i64 = text.parse().map_err(|error| {
                anyhow::anyhow!("requires a signed {bits}-bit integer: {error}")
            })?;
            ensure!(
                *bits == 64 || (n >= -(1i64 << (bits - 1)) && n < (1i64 << (bits - 1))),
                "signed {bits}-bit integer overflow"
            );
            // PostgreSQL must retain exact narrow types inside records.
            #[cfg(feature = "postgres")]
            {
                Ok(match bits {
                    32 => Bound::Int32(n as i32),
                    64 => Bound::Int(n),
                    _ => Bound::Text(n.to_string()),
                })
            }
            #[cfg(not(feature = "postgres"))]
            {
                Ok(Bound::Int(n))
            }
        }
        Kind::Unsigned(bits) => {
            ensure!(
                !value.quoted,
                "requires an unquoted unsigned {bits}-bit integer"
            );
            let n: u64 = text.parse()?;
            ensure!(*bits == 64 || n < (1u64 << bits), "integer overflow");
            #[cfg(feature = "mysql")]
            {
                Ok(Bound::UInt(n))
            }
            #[cfg(not(feature = "mysql"))]
            {
                Ok(Bound::Text(n.to_string()))
            }
        }
        Kind::Bool => {
            ensure!(
                !value.quoted && matches!(text.as_str(), "true" | "false"),
                "requires unquoted true or false"
            );
            #[cfg(feature = "postgres")]
            {
                Ok(Bound::Text(text.clone()))
            }
            #[cfg(not(feature = "postgres"))]
            {
                Ok(Bound::Int(i64::from(text == "true")))
            }
        }
        Kind::Float(bits) => {
            ensure!(!value.quoted, "requires an unquoted float");
            let n: f64 = text.parse()?;
            ensure!(
                n.is_finite() && (*bits != 32 || (n as f32).is_finite()),
                "float must be finite and within range"
            );
            #[cfg(feature = "postgres")]
            {
                Ok(Bound::Text(text.clone()))
            }
            #[cfg(feature = "mysql")]
            {
                Ok(if *bits == 32 {
                    Bound::Float32(n as f32)
                } else {
                    Bound::Float(n)
                })
            }
            #[cfg(feature = "sqlite")]
            {
                Ok(Bound::Float(n))
            }
        }
        Kind::Numeric => {
            ensure!(!value.quoted, "requires an unquoted decimal");
            validate_decimal(text)?;
            #[cfg(feature = "numeric")]
            {
                let _: bigdecimal::BigDecimal = text.parse()?;
            }
            #[cfg(feature = "sqlite")]
            {
                let n: f64 = text.parse()?;
                ensure!(n.is_finite(), "numeric out of range");
                Ok(Bound::Float(n))
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Ok(Bound::Text(text.clone()))
            }
        }
        Kind::Binary => {
            ensure!(value.quoted, "requires quoted hexadecimal bytes (0x...)");
            let hex = text
                .strip_prefix("0x")
                .or_else(|| text.strip_prefix("\\x"))
                .ok_or_else(|| anyhow::anyhow!("expected 0x hexadecimal prefix"))?;
            ensure!(
                hex.len() % 2 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid hexadecimal bytes"
            );
            Ok(Bound::Bytes(
                hex.as_bytes()
                    .chunks(2)
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect(),
            ))
        }
        Kind::Json => {
            let _: serde_json::Value = serde_json::from_str(text)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Record(types) => {
            ensure!(value.quoted, "requires a quoted PostgreSQL record literal");
            let fields = record_fields(text)?;
            ensure!(
                fields.len() == types.len(),
                "record requires {} fields",
                types.len()
            );
            Ok(Bound::Record(
                types
                    .iter()
                    .cloned()
                    .zip(fields)
                    .map(|(ty, field)| {
                        let value = match field {
                            None => Bound::Null,
                            Some(text) => parse(
                                &ty,
                                &Value {
                                    text,
                                    quoted: !matches!(
                                        ty.kind,
                                        Kind::Signed(_)
                                            | Kind::Unsigned(_)
                                            | Kind::Float(_)
                                            | Kind::Numeric
                                            | Kind::Bool
                                    ),
                                },
                            )?,
                        };
                        Ok((ty, value))
                    })
                    .collect::<Result<_>>()?,
            ))
        }
        Kind::Date | Kind::Time | Kind::Timestamp | Kind::Timestamptz => {
            ensure!(value.quoted, "requires a quoted ISO date/time");
            validate_temporal(&ty.kind, text)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Uuid => {
            ensure!(value.quoted, "requires a quoted UUID");
            ensure!(
                text.len() == 36
                    && text
                        .char_indices()
                        .all(|(i, c)| if [8, 13, 18, 23].contains(&i) {
                            c == '-'
                        } else {
                            c.is_ascii_hexdigit()
                        }),
                "invalid UUID"
            );
            #[cfg(feature = "uuid")]
            {
                let _: uuid::Uuid = text.parse()?;
            }
            Ok(Bound::Text(text.clone()))
        }
        Kind::Network => {
            ensure!(value.quoted, "requires a quoted network address");
            let (ip, prefix) = text
                .split_once('/')
                .map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
            let ip: std::net::IpAddr = ip.parse()?;
            if let Some(prefix) = prefix {
                let prefix: u8 = prefix.parse()?;
                ensure!(
                    prefix <= if ip.is_ipv4() { 32 } else { 128 },
                    "invalid network prefix"
                );
            }
            Ok(Bound::Text(text.clone()))
        }
        Kind::Mac(size) => {
            ensure!(value.quoted, "requires a quoted MAC address");
            let digits: String = text
                .chars()
                .filter(|c| !matches!(c, ':' | '-' | '.'))
                .collect();
            ensure!(
                digits.len() == size * 2 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid MAC address"
            );
            Ok(Bound::Text(
                digits
                    .as_bytes()
                    .chunks(2)
                    .map(|b| std::str::from_utf8(b).unwrap())
                    .collect::<Vec<_>>()
                    .join(":"),
            ))
        }
        Kind::Lsn => {
            ensure!(value.quoted, "requires a quoted WAL position");
            let (hi, lo) = text
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("invalid WAL position"))?;
            u32::from_str_radix(hi, 16)?;
            u32::from_str_radix(lo, 16)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Char => {
            #[cfg(feature = "postgres")]
            {
                let byte = if value.quoted {
                    ensure!(
                        text.len() == 1,
                        "requires one quoted byte or an unquoted unsigned 8-bit integer"
                    );
                    text.as_bytes()[0]
                } else {
                    text.parse::<u8>()?
                };
                Ok(Bound::CChar(byte))
            }
            #[cfg(not(feature = "postgres"))]
            {
                bail!("CChar is PostgreSQL-only");
            }
        }
        Kind::Array(_) | Kind::Range(_) | Kind::Multirange(_) | Kind::Other => {
            ensure!(value.quoted, "requires a quoted PostgreSQL native literal");
            Ok(Bound::Text(text.clone()))
        }
        Kind::Text | Kind::Jsonb => Ok(Bound::Text(text.clone())),
    }
}
fn validate_decimal(s: &str) -> Result<()> {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exp) = s
        .split_once(['e', 'E'])
        .map_or((s, None), |(a, b)| (a, Some(b)));
    let mut dot = false;
    let mut digits = 0;
    for b in mantissa.bytes() {
        if b == b'.' && !dot {
            dot = true;
        } else {
            ensure!(b.is_ascii_digit(), "invalid decimal");
            digits += 1;
        }
    }
    ensure!(digits > 0, "invalid decimal");
    if let Some(exp) = exp {
        let _: i32 = exp.parse()?;
    }
    Ok(())
}
fn validate_temporal(kind: &Kind, text: &str) -> Result<()> {
    #[cfg(feature = "chrono")]
    {
        match kind {
            Kind::Date => {
                chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")?;
            }
            Kind::Time => {
                chrono::NaiveTime::parse_from_str(text, "%H:%M:%S%.f")?;
            }
            Kind::Timestamp => {
                chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").or_else(
                    |_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"),
                )?;
            }
            Kind::Timestamptz => {
                chrono::DateTime::parse_from_rfc3339(text)?;
            }
            _ => unreachable!(),
        }
    }
    #[cfg(all(feature = "time", not(feature = "chrono")))]
    {
        use time::format_description::well_known::{Iso8601, Rfc3339};
        match kind {
            Kind::Date => {
                time::Date::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Time => {
                time::Time::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Timestamp => {
                time::PrimitiveDateTime::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Timestamptz => {
                time::OffsetDateTime::parse(text, &Rfc3339)?;
            }
            _ => unreachable!(),
        }
    }
    #[cfg(not(any(feature = "chrono", feature = "time")))]
    {
        let _ = kind;
        ensure!(!text.is_empty(), "empty date/time literal");
    }
    Ok(())
}
// PostgreSQL composite text format: empty unquoted fields are NULL, doubled
// quotes/backslashes escape characters inside a quoted field.
fn record_fields(text: &str) -> Result<Vec<Option<String>>> {
    let body = text
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| anyhow::anyhow!("invalid record literal"))?;
    let mut fields = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => value.push(
                chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("unfinished record escape"))?,
            ),
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                value.push('"');
            }
            '"' => {
                quoted = !quoted;
                was_quoted = true;
            }
            ',' if !quoted => {
                fields.push(if value.is_empty() && !was_quoted {
                    None
                } else {
                    Some(std::mem::take(&mut value))
                });
                was_quoted = false;
            }
            c => value.push(c),
        }
    }
    ensure!(!quoted, "unterminated record string");
    fields.push(if value.is_empty() && !was_quoted {
        None
    } else {
        Some(value)
    });
    Ok(fields)
}

#[cfg(feature = "mysql")]
pub(super) fn mysql_decimal_type(text: &str) -> Result<String> {
    validate_decimal(text)?;
    let text = text.trim_start_matches(['-', '+']);
    let (mantissa, exp) = text
        .split_once(['e', 'E'])
        .map_or((text, 0), |(a, b)| (a, b.parse::<i32>().unwrap()));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let scale = (fraction.len() as i64 - i64::from(exp)).max(0);
    let integers = (whole.trim_start_matches('0').len() as i64 + i64::from(exp)).max(0);
    ensure!(
        scale <= 30 && integers + scale <= 65,
        "decimal exceeds MySQL DECIMAL precision/scale"
    );
    Ok(format!("DECIMAL(65,{scale})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_preserve_null_empty_strings_delimiters_and_escapes() {
        assert_eq!(
            record_fields(r#"(,"", "a,b","a""b","a\\b")"#).unwrap(),
            vec![
                None,
                Some("".into()),
                Some(" a,b".into()),
                Some("a\"b".into()),
                Some("a\\b".into())
            ]
        );
        assert!(record_fields("(\"unfinished)").is_err());
        assert!(record_fields("a,b").is_err());
    }
    #[test]
    fn decimals_validate_without_float_conversion() {
        for text in ["0", "-0.01", "1e+100", "12345678901234567890.123456789"] {
            validate_decimal(text).unwrap();
        }
        for text in [
            "",
            ".",
            "NaN",
            "inf",
            "1.2.3",
            "1e",
            "1e9999999999",
            "1; DROP TABLE x",
        ] {
            assert!(validate_decimal(text).is_err(), "{text}");
        }
    }
    #[cfg(any(feature = "chrono", feature = "time"))]
    #[test]
    fn temporal_formats_are_validated_with_either_integration() {
        for (kind, text) in [
            (Kind::Date, "2026-10-17"),
            (Kind::Time, "12:34:56.123456"),
            (Kind::Timestamp, "2026-10-17T12:34:56"),
            (Kind::Timestamptz, "2026-10-17T12:34:56+08:00"),
        ] {
            validate_temporal(&kind, text).unwrap();
        }
        assert!(validate_temporal(&Kind::Date, "2026-02-30").is_err());
    }
}

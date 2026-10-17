//! Canonical SQLite temporal encodings for supported Rust types.

use anyhow::Result;

/// SQLite stores temporal values as text. Match Diesel's representation for
/// the model's temporal crate, including its fractional precision and UTC
/// normalization, so comparisons can keep using the column's index.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "enable the matching `chrono` or `time` feature on diesel-aip-160 for SQLite temporal model type `{Self}`"
)]
pub trait SqliteTemporal {
    fn canonical(text: &str) -> Result<String>;
}
impl SqliteTemporal for String {
    fn canonical(text: &str) -> Result<String> {
        Ok(text.to_owned())
    }
}
#[cfg(feature = "chrono")]
mod sqlite_chrono {
    use super::*;
    impl SqliteTemporal for chrono::NaiveDate {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%Y-%m-%d")?
                .format("%F")
                .to_string())
        }
    }
    impl SqliteTemporal for chrono::NaiveTime {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%H:%M:%S%.f")?
                .format("%T%.f")
                .to_string())
        }
    }
    impl SqliteTemporal for chrono::NaiveDateTime {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| Self::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(text).map(|v| v.naive_utc()))?
                .format("%F %T%.f")
                .to_string())
        }
    }
    impl<Tz: chrono::TimeZone> SqliteTemporal for chrono::DateTime<Tz> {
        fn canonical(text: &str) -> Result<String> {
            Ok(chrono::DateTime::parse_from_rfc3339(text)?
                .with_timezone(&chrono::Utc)
                .format("%F %T%.f%:z")
                .to_string())
        }
    }
}
#[cfg(feature = "time")]
mod sqlite_time {
    use super::*;
    use time::format_description::parse_borrowed;
    use time::format_description::well_known::{Iso8601, Rfc3339};
    impl SqliteTemporal for time::Date {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse(text, &Iso8601::DEFAULT)?.to_string())
        }
    }
    fn format(
        date: bool,
        fraction: bool,
        offset: bool,
    ) -> Result<Vec<time::format_description::BorrowedFormatItem<'static>>> {
        let pattern = match (date, fraction, offset) {
            (false, false, _) => "[hour]:[minute]:[second]",
            (false, true, _) => "[hour]:[minute]:[second].[subsecond]",
            (true, false, false) => "[year]-[month]-[day] [hour]:[minute]:[second]",
            (true, true, false) => "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]",
            (true, false, true) => {
                "[year]-[month]-[day] [hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
            }
            (true, true, true) => {
                "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond][offset_hour sign:mandatory]:[offset_minute]"
            }
        };
        Ok(parse_borrowed::<2>(pattern)?)
    }
    impl SqliteTemporal for time::Time {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(text, &Iso8601::DEFAULT)?;
            // Match Diesel 2.3's SQLite Time serializer, including its microsecond guard.
            Ok(value.format(&format(false, value.microsecond() != 0, false)?)?)
        }
    }
    impl SqliteTemporal for time::PrimitiveDateTime {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(&text.replace(' ', "T"), &Iso8601::DEFAULT).or_else(|_| {
                time::OffsetDateTime::parse(text, &Rfc3339).map(|value| {
                    let value = value.to_offset(time::UtcOffset::UTC);
                    time::PrimitiveDateTime::new(value.date(), value.time())
                })
            })?;
            Ok(value.format(&format(true, value.nanosecond() != 0, false)?)?)
        }
    }
    impl SqliteTemporal for time::OffsetDateTime {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(text, &Rfc3339)?.to_offset(time::UtcOffset::UTC);
            Ok(value.format(&format(true, value.nanosecond() != 0, true)?)?)
        }
    }
}

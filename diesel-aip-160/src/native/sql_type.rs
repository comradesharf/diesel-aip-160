//! Sealed SQL type metadata and schema-driven Rust type dispatch.

#[cfg(feature = "sqlite")]
use super::SqliteTemporal;
use anyhow::Result;
use diesel::sql_types::{self as st, SqlType};

#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum Kind {
    Text,
    Bool,
    Signed(u8),
    Unsigned(u8),
    Float(u8),
    Numeric,
    Binary,
    Date,
    Time,
    Timestamp,
    Timestamptz,
    Json,
    Jsonb,
    Uuid,
    Network,
    Mac(usize),
    Lsn,
    Char,
    Other,
    Array(Box<Type>),
    Range(Box<Type>),
    Multirange(Box<Type>),
    Record(Vec<Type>),
}
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct Type {
    pub name: String,
    pub kind: Kind,
    pub nullable: bool,
    pub unsigned: bool,
}
impl Type {
    fn new(name: &str, kind: Kind) -> Self {
        Self {
            name: name.into(),
            kind,
            nullable: false,
            unsigned: false,
        }
    }
}
mod sealed {
    pub trait Sealed {}
}
/// Implemented for Diesel's built-in SQL types. User-defined types use adapters.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "unsupported Diesel SQL type `{Self}`; use #[aip160(adapter = MyAdapter)] or #[aip160(skip)] for application-defined SQL types"
)]
pub trait NativeSqlType: SqlType + sealed::Sealed {
    fn native_type() -> Type;
}
#[doc(hidden)]
pub trait NativeType<T>: NativeSqlType {
    fn normalize(text: String) -> Result<String> {
        Ok(text)
    }
    fn path(field: &str, column: &str) -> Option<Vec<String>> {
        (field == column).then(Vec::new)
    }
}
macro_rules! scalar {
    ($sql:ty, $name:expr, $kind:expr) => {
        impl sealed::Sealed for $sql {}
        impl NativeSqlType for $sql {
            fn native_type() -> Type {
                Type::new($name, $kind)
            }
        }
        impl<T> NativeType<T> for $sql {}
    };
}
scalar!(st::Text, "text", Kind::Text);
scalar!(st::Bool, "boolean", Kind::Bool);
scalar!(st::SmallInt, "smallint", Kind::Signed(16));
scalar!(st::Integer, "integer", Kind::Signed(32));
scalar!(st::BigInt, "bigint", Kind::Signed(64));
scalar!(st::Float, "real", Kind::Float(32));
scalar!(st::Double, "double precision", Kind::Float(64));
scalar!(st::Numeric, "numeric", Kind::Numeric);
scalar!(st::Binary, "bytea", Kind::Binary);
macro_rules! temporal {
    ($sql:ty, $name:expr, $kind:expr) => {
        impl sealed::Sealed for $sql {}
        impl NativeSqlType for $sql {
            fn native_type() -> Type {
                Type::new($name, $kind)
            }
        }
        #[cfg(not(feature = "sqlite"))]
        impl<T> NativeType<T> for $sql {}
        #[cfg(feature = "sqlite")]
        impl<T: SqliteTemporal> NativeType<T> for $sql {
            fn normalize(text: String) -> Result<String> {
                T::canonical(&text)
            }
        }
    };
}
temporal!(st::Date, "date", Kind::Date);
temporal!(st::Time, "time", Kind::Time);
temporal!(st::Timestamp, "timestamp", Kind::Timestamp);
scalar!(st::Json, "json", Kind::Json);
#[cfg(feature = "postgres")]
scalar!(st::Interval, "interval", Kind::Other);
#[cfg(feature = "mysql")]
scalar!(st::TinyInt, "tinyint", Kind::Signed(8));
#[cfg(feature = "sqlite")]
temporal!(st::TimestamptzSqlite, "text", Kind::Timestamptz);
#[cfg(feature = "sqlite")]
scalar!(st::Jsonb, "jsonb", Kind::Json);
#[cfg(feature = "mysql")]
scalar!(st::Datetime, "datetime", Kind::Timestamp);
#[cfg(feature = "postgres")]
mod pg {
    use super::*;
    scalar!(st::Uuid, "uuid", Kind::Uuid);
    scalar!(st::Timestamptz, "timestamptz", Kind::Timestamptz);
    scalar!(st::Oid, "oid", Kind::Unsigned(32));
    scalar!(st::Money, "money", Kind::Other);
    scalar!(st::Inet, "inet", Kind::Network);
    scalar!(st::Cidr, "cidr", Kind::Network);
    scalar!(st::MacAddr, "macaddr", Kind::Mac(6));
    scalar!(st::MacAddr8, "macaddr8", Kind::Mac(8));
    scalar!(st::CChar, "\"char\"", Kind::Char);
    scalar!(st::Citext, "citext", Kind::Text);
    scalar!(st::PgLsn, "pg_lsn", Kind::Lsn);
    impl sealed::Sealed for st::Jsonb {}
    impl NativeSqlType for st::Jsonb {
        fn native_type() -> Type {
            Type::new("jsonb", Kind::Jsonb)
        }
    }
    impl<T: crate::JsonbPath> NativeType<T> for st::Jsonb {
        fn path(field: &str, column: &str) -> Option<Vec<String>> {
            crate::json_path::<T>(field, column)
        }
    }
    macro_rules! container {
        ($sql:ident, $kind:ident, $name:expr) => {
            impl<S: NativeSqlType + 'static> sealed::Sealed for st::$sql<S> {}
            impl<S: NativeSqlType + 'static> NativeSqlType for st::$sql<S> {
                fn native_type() -> Type {
                    let inner = S::native_type();
                    Type::new(&$name(&inner), Kind::$kind(Box::new(inner)))
                }
            }
            impl<T, S: NativeSqlType + 'static> NativeType<T> for st::$sql<S> {}
        };
    }
    container!(Array, Array, |inner: &Type| format!("{}[]", inner.name));
    fn range_name(inner: &Type, multi: bool) -> String {
        let base = match inner.name.as_str() {
            "integer" => "int4",
            "bigint" => "int8",
            "numeric" => "num",
            "date" => "date",
            "timestamp" => "ts",
            "timestamptz" => "tstz",
            _ => "unsupported",
        };
        format!("{base}{}range", if multi { "multi" } else { "" })
    }
    container!(Range, Range, |inner: &Type| range_name(inner, false));
    container!(Multirange, Multirange, |inner: &Type| range_name(
        inner, true
    ));
    pub trait TupleTypes {
        fn types() -> Vec<Type>;
    }
    macro_rules! tuple {
        ($($t:ident),+) => { impl<$($t: NativeSqlType),+> TupleTypes for ($($t,)+) {
            fn types() -> Vec<Type> { vec![$($t::native_type()),+] }
        } };
    }
    macro_rules! tuples {
        ($head:ident $(,$tail:ident)*) => { tuple!($head $(,$tail)*); tuples!($($tail),*); };
        () => {};
    }
    tuples!(
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC,
        AD, AE, AF
    );
    impl<S: TupleTypes + 'static> sealed::Sealed for st::Record<S> {}
    impl<S: TupleTypes + 'static> NativeSqlType for st::Record<S> {
        fn native_type() -> Type {
            Type::new("record", Kind::Record(S::types()))
        }
    }
    impl<T, S: TupleTypes + 'static> NativeType<T> for st::Record<S> {}
}
#[cfg(feature = "mysql")]
mod mysql {
    use super::*;
    macro_rules! unsigned {
        ($sql:ty, $bits:expr) => {
            impl sealed::Sealed for st::Unsigned<$sql> {}
            impl NativeSqlType for st::Unsigned<$sql> {
                fn native_type() -> Type {
                    Type::new("unsigned", Kind::Unsigned($bits))
                }
            }
            impl<T> NativeType<T> for st::Unsigned<$sql> {}
        };
    }
    unsigned!(st::TinyInt, 8);
    unsigned!(st::SmallInt, 16);
    unsigned!(st::Integer, 32);
    unsigned!(st::BigInt, 64);
    macro_rules! unsigned_decimal {
        ($sql:ty) => {
            impl sealed::Sealed for st::Unsigned<$sql> {}
            impl NativeSqlType for st::Unsigned<$sql> {
                fn native_type() -> Type {
                    let mut ty = <$sql>::native_type();
                    ty.unsigned = true;
                    ty
                }
            }
            impl<T> NativeType<T> for st::Unsigned<$sql> {}
        };
    }
    unsigned_decimal!(st::Float);
    unsigned_decimal!(st::Double);
    unsigned_decimal!(st::Numeric);
}
impl<S: NativeSqlType> sealed::Sealed for st::Nullable<S> {}
impl<S: NativeSqlType> NativeSqlType for st::Nullable<S> {
    fn native_type() -> Type {
        let mut ty = S::native_type();
        ty.nullable = true;
        ty
    }
}
impl<T, S: NativeType<T>> NativeType<T> for st::Nullable<S> {
    fn normalize(text: String) -> Result<String> {
        S::normalize(text)
    }
    fn path(field: &str, column: &str) -> Option<Vec<String>> {
        S::path(field, column)
    }
}

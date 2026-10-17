//! Derive macros for AIP-160 filtering with Diesel.
//!
//! Prefer the macros re-exported by `diesel_aip_160`, which also provides the
//! parser, compiler, and runtime helpers required by generated code.
//! [`Aip160Filter`] requires a named-field struct with
//! `#[diesel(table_name = ...)]` and one database backend enabled on the runtime
//! crate. [`Aip160Jsonb`] maps Rust field names to stored JSONB keys.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericArgument, PathArguments, Type, parse_macro_input};

mod pg_enum;

/// Generate PostgreSQL enum codecs for a local newtype around a dependency type.
///
/// Requires `#[diesel(sql_type = SqlType)]` and
/// `#[aip160(value_type = i16, from_value = decode, to_value = encode)]`.
/// `decode` takes the parsed value and returns `Result<Inner, E>`; `encode`
/// takes `&Inner` and returns `Result<ValueType, E>`. Errors must convert to
/// `Box<dyn std::error::Error + Send + Sync>`. Values use `FromStr`/`Display`
/// to read/write PostgreSQL enum labels. Enable the runtime's `postgres` feature.
/// Derive Diesel's `AsExpression` and `FromSqlRow` separately. `Aip160Filter`
/// automatically reuses these conversions for wrapper fields (including `Option`)
/// without a field adapter. Ordering follows PostgreSQL enum declaration order.
#[proc_macro_derive(Aip160PgEnum, attributes(aip160, diesel))]
pub fn derive_aip160_pg_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pg_enum::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generate a typed filter compiler and a `compile_filter` method for a Diesel model.
///
/// Use `#[diesel(table_name = ...)]` to specify the table and `#[aip160(skip)]`
/// to exclude fields from filtering.
#[proc_macro_derive(Aip160Filter, attributes(aip160, diesel))]
pub fn derive_aip160_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Map Rust field names to their serialized JSON keys for filter traversal.
#[proc_macro_derive(Aip160Jsonb, attributes(serde))]
pub fn derive_aip160_jsonb(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_jsonb(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_jsonb(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Aip160Jsonb requires a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Aip160Jsonb requires named fields",
        ));
    };
    for attribute in &input.attrs {
        if attribute.path().is_ident("serde") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename_all") {
                    return Err(meta.error("Aip160Jsonb does not support serde rename_all"));
                }
                let _ = meta.input.parse::<proc_macro2::TokenStream>();
                Ok(())
            })?;
        }
    }
    let mut mappings = Vec::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let mut stored = name.clone();
        for attribute in &field.attrs {
            if attribute.path().is_ident("serde") {
                attribute.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        stored = meta.value()?.parse::<syn::LitStr>()?.value();
                    } else {
                        let _ = meta.input.parse::<proc_macro2::TokenStream>();
                    }
                    Ok(())
                })?;
            }
        }
        mappings.push(quote!(#name => Some(#stored),));
    }
    let model = &input.ident;
    Ok(quote! {
        impl ::diesel_aip_160::JsonbPath for #model {
            fn stored_key(field: &str) -> Option<&str> {
                match field {
                    #(#mappings)*
                    _ => None,
                }
            }
        }
    })
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut table = None;
    for attribute in &input.attrs {
        if attribute.path().is_ident("diesel") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("table_name") {
                    table = Some(meta.value()?.parse::<syn::Path>()?);
                } else {
                    let _ = meta.input.parse::<proc_macro2::TokenStream>();
                }
                Ok(())
            })?;
        }
    }
    let table = table.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "Aip160Filter requires #[diesel(table_name = ...)]",
        )
    })?;
    let compiler = format_ident!("{}FilterCompiler", input.ident);
    let model = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Aip160Filter requires a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Aip160Filter requires named fields",
        ));
    };
    let mut types = Vec::new();
    let mut natives = Vec::new();
    let mut customs = Vec::new();
    let mut texts = Vec::new();
    let mut ilikes = Vec::new();
    for field in &fields.named {
        let (skip, adapter) = field_options(field)?;
        if skip {
            continue;
        }
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let (ty, nullable) = unwrap_option(&field.ty);
        let column = quote!(#table::#ident);
        if let Some(adapter) = adapter {
            types.push(quote!(if field == #name { return Ok(::diesel_aip_160::compiler::FieldType::Custom { nullable: #nullable }); }));
            customs.push(quote!(#name => <#adapter as ::diesel_aip_160::diesel_helpers::FieldAdapter<#column>>::compare(#column, operator, value),));
            continue;
        }
        types.push(quote!(if let Some(ty) = <#ty>::__aip160_field_type(#column, field, #name) { return Ok(ty); }));
        natives.push(
            quote!(if <#ty>::__aip160_field_type(#column, field, #name).is_some() {
                return <#ty>::__aip160_compare(#column, field, #name, operator, value);
            }),
        );
        texts.push(quote!(if <#ty>::__aip160_is_text(#column) { fields.push(#name); }));
        ilikes.push(quote!(#name => <#ty>::__aip160_like(#column, pattern, true),));
    }
    Ok(quote! {
        impl #model {
            pub fn compile_filter(source: &str) -> ::diesel_aip_160::anyhow::Result<Option<::diesel_aip_160::diesel_helpers::Predicate<#table::table>>> {
                ::diesel_aip_160::compiler::compile(source, &#compiler)
            }
        }
        pub struct #compiler;
        impl ::diesel_aip_160::compiler::Aip160FilterCompiler for #compiler {
            type Predicate = ::diesel_aip_160::diesel_helpers::Predicate<#table::table>;
            fn custom_compare(&self, field: &str, operator: ::diesel_aip_160::compiler::ComparisonOperator,
                value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                match field { #(#customs)* _ => ::diesel_aip_160::anyhow::bail!("unknown adapter field `{field}`"), }
            }
            fn native_compare(&self, field: &str, operator: ::diesel_aip_160::parser::Comparator,
                value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                #[allow(unused_imports)]
                use ::diesel_aip_160::native::FilterField as _;
                #(#natives)*
                ::diesel_aip_160::anyhow::bail!("unknown filter field `{field}`")
            }
            fn field_type(&self, field: &str) -> ::diesel_aip_160::anyhow::Result<::diesel_aip_160::compiler::FieldType> {
                #[allow(unused_imports)]
                use ::diesel_aip_160::native::FilterField as _;
                #(#types)*
                ::diesel_aip_160::anyhow::bail!("unknown filter field `{field}`")
            }
            fn text_fields(&self) -> &'static [&'static str] {
                #[allow(unused_imports)]
                use ::diesel_aip_160::native::FilterField as _;
                static FIELDS: ::std::sync::OnceLock<Vec<&'static str>> = ::std::sync::OnceLock::new();
                FIELDS.get_or_init(|| { let mut fields = Vec::new(); #(#texts)* fields }).as_slice()
            }
            fn ilike(&self, field: &str, pattern: String) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                #[allow(unused_imports)]
                use ::diesel_aip_160::native::FilterField as _;
                match field { #(#ilikes)* _ => ::diesel_aip_160::anyhow::bail!("unknown string field `{field}`"), }
            }
            fn constant(&self, value: bool) -> Self::Predicate {
                use ::diesel_aip_160::diesel::IntoSql;
                Box::new(value.into_sql::<::diesel_aip_160::diesel::sql_types::Bool>())
            }
            fn and(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate {
                use ::diesel_aip_160::diesel::BoolExpressionMethods;
                Box::new(left.and(right))
            }
            fn or(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate {
                use ::diesel_aip_160::diesel::BoolExpressionMethods;
                Box::new(left.or(right))
            }
            fn not(&self, predicate: Self::Predicate) -> Self::Predicate {
                Box::new(::diesel_aip_160::diesel::dsl::not(predicate))
            }
        }
    })
}

fn field_options(field: &syn::Field) -> syn::Result<(bool, Option<syn::Path>)> {
    let mut skip = false;
    let mut adapter = None;
    for attribute in &field.attrs {
        if attribute.path().is_ident("aip160") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                    Ok(())
                } else if meta.path.is_ident("adapter") {
                    if adapter.is_some() {
                        return Err(meta.error("duplicate adapter"));
                    }
                    adapter = Some(meta.value()?.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("expected skip or adapter = path::Adapter"))
                }
            })?;
        }
    }
    if skip && adapter.is_some() {
        return Err(syn::Error::new_spanned(
            field,
            "skip and adapter cannot be combined",
        ));
    }
    Ok((skip, adapter))
}

fn unwrap_option(ty: &Type) -> (&Type, bool) {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == "Option"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return (inner, true);
    }
    (ty, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact(tokens: proc_macro2::TokenStream) -> String {
        tokens.to_string().replace(' ', "")
    }

    #[test]
    fn jsonb_maps_rust_names_to_serialized_keys() {
        let input = syn::parse_quote! {
            struct Details {
                ordinary: String,
                #[serde(rename = "stored_name")]
                renamed: i32,
            }
        };
        let output = compact(expand_jsonb(input).unwrap());
        assert!(output.contains("\"ordinary\"=>Some(\"ordinary\")"));
        assert!(output.contains("\"renamed\"=>Some(\"stored_name\")"));
        assert!(output.contains("_=>None"));
    }

    #[test]
    fn jsonb_rejects_rename_all_and_non_named_structs() {
        let renamed = syn::parse_quote! {
            #[serde(rename_all = "camelCase")]
            struct Details { field_name: String }
        };
        assert!(
            expand_jsonb(renamed)
                .unwrap_err()
                .to_string()
                .contains("rename_all")
        );

        let tuple = syn::parse_quote! { struct Details(String); };
        assert!(
            expand_jsonb(tuple)
                .unwrap_err()
                .to_string()
                .contains("named fields")
        );
    }

    #[test]
    fn filter_generates_supported_fields_and_skips_marked_fields() {
        let input = syn::parse_quote! {
            #[diesel(table_name = records)]
            struct Record {
                name: String,
                count: i32,
                optional_name: Option<String>,
                details: Details,
                #[aip160(skip)]
                ignored: String,
            }
        };
        let output = compact(expand(input).unwrap());
        assert!(output.contains("RecordFilterCompiler"));
        assert!(output.contains("<String>::__aip160_field_type(records::name"));
        assert!(output.contains("<String>::__aip160_field_type(records::optional_name"));
        assert!(output.contains("<i32>::__aip160_field_type(records::count"));
        assert!(output.contains("<Details>::__aip160_field_type(records::details"));
        assert!(!output.contains("\"ignored\""));
    }

    #[test]
    fn adapters_and_integer_widths_are_explicit() {
        let input = syn::parse_quote! {
            #[diesel(table_name = records)]
            struct Record {
                small: Option<i32>,
                large: i64,
                optional: Option<i64>,
                #[aip160(adapter = crate::CodeAdapter)]
                code: Code,
            }
        };
        let output = compact(expand(input).unwrap());
        assert!(output.contains("<i32>::__aip160_field_type(records::small"));
        assert!(output.contains("<i64>::__aip160_field_type(records::large"));
        assert!(output.contains("<i64>::__aip160_field_type(records::optional"));
        assert!(output.contains("FieldAdapter<records::code>"));
        assert!(!output.contains("json_path"));
    }

    #[test]
    fn unsupported_types_and_attributes_have_actionable_errors() {
        let input = syn::parse_quote! {
            #[diesel(table_name = records)]
            struct Record { enabled: bool }
        };
        assert!(expand(input).is_ok());
        for attributes in [
            quote!(skip, adapter = Adapter),
            quote!(adapter = A, adapter = B),
            quote!(unknown),
        ] {
            let input = syn::parse_quote! {
                #[diesel(table_name = records)]
                struct Record { #[aip160(#attributes)] code: Code }
            };
            assert!(expand(input).is_err());
        }
    }

    #[test]
    fn filter_requires_a_table_name() {
        let input = syn::parse_quote! { struct Record { name: String } };
        assert!(
            expand(input)
                .unwrap_err()
                .to_string()
                .contains("table_name")
        );
    }
}

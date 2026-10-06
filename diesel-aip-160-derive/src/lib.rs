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
    let mut comparisons = Vec::new();
    let mut types = Vec::new();
    let mut likes = Vec::new();
    let mut text_names = Vec::new();
    let mut json_types = Vec::new();
    let mut json_comparisons = Vec::new();
    let mut json_containments = Vec::new();
    let mut json_presences = Vec::new();
    let mut jsonb_columns = Vec::new();
    for field in &fields.named {
        let mut skip = false;
        for attribute in &field.attrs {
            if attribute.path().is_ident("aip160") {
                attribute.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                        Ok(())
                    } else {
                        Err(meta.error("expected skip"))
                    }
                })?;
            }
        }
        if skip {
            continue;
        }
        let (ty, _) = unwrap_option(&field.ty);
        if !is_type(ty, "String") && !is_type(ty, "i32") {
            jsonb_columns.push((field.ident.as_ref().unwrap().clone(), ty.clone()));
        }
    }
    for (ident, ty) in &jsonb_columns {
        let name = ident.to_string();
        let path = quote!(::diesel_aip_160::json_path::<#ty>(field, #name));
        json_types.push(
            quote!(if #path.is_some() { return Ok(::diesel_aip_160::compiler::FieldType::Jsonb); }),
        );
        json_comparisons.push(quote!(if let Some(path) = #path { return Ok(::diesel_aip_160::diesel_helpers::jsonb_compare(#table::#ident, path, operator, value)); }));
        json_containments.push(quote!(if let Some(path) = #path { return Ok(::diesel_aip_160::diesel_helpers::jsonb_contains(#table::#ident, path, value)); }));
        json_presences.push(quote!(if let Some(path) = #path { return Ok(::diesel_aip_160::diesel_helpers::jsonb_presence(#table::#ident, path)); }));
    }
    for field in &fields.named {
        if field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("aip160"))
        {
            continue;
        }
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let (ty, nullable) = unwrap_option(&field.ty);
        let kind = if is_type(ty, "String") {
            "string"
        } else if is_type(ty, "i32") {
            "integer"
        } else {
            continue;
        };
        let column = quote!(#table::#ident);
        if kind == "string" {
            types.push(
                quote!(#name => Ok(::diesel_aip_160::compiler::FieldType::String { nullable: #nullable }),),
            );
            text_names.push(name.clone());
            comparisons.push(quote! {
                (#name, ::diesel_aip_160::compiler::FilterLiteral::String(value)) =>
                    ::diesel_aip_160::diesel_helpers::compare_string_column(#column, operator, Some(value)),
            });
            if nullable {
                comparisons.push(quote! {
                    (#name, ::diesel_aip_160::compiler::FilterLiteral::Null) =>
                        ::diesel_aip_160::diesel_helpers::compare_string_column(#column, operator, None),
                });
            }
            comparisons.push(quote! {
                (#name, value) => ::diesel_aip_160::anyhow::bail!("field `{}` requires a string value, received {value:?}", #name),
            });
            let expression = if nullable {
                quote!(#column.like(pattern).escape('\\').assume_not_null())
            } else {
                quote!(#column.like(pattern).escape('\\'))
            };
            likes.push(quote!(#name => Ok(Box::new(#expression)),));
        } else {
            types.push(quote!(#name => Ok(::diesel_aip_160::compiler::FieldType::Int32),));
            comparisons.push(quote! {
                (#name, ::diesel_aip_160::compiler::FilterLiteral::Int(value)) =>
                    ::diesel_aip_160::diesel_helpers::compare_integer_column(#column, operator, Some(value)),
                (#name, value) => ::diesel_aip_160::anyhow::bail!("field `{}` requires a signed 32-bit integer value, received {value:?}", #name),
            });
        }
    }
    let predicate = format_ident!("{}Predicate", compiler);
    Ok(quote! {
        impl #model {
            pub fn compile_filter(source: &str) -> ::diesel_aip_160::anyhow::Result<Option<::diesel_aip_160::diesel_helpers::Predicate<#table::table>>> {
                ::diesel_aip_160::compiler::compile(source, &#compiler)
            }
        }
        pub struct #compiler;
        type #predicate = ::diesel_aip_160::diesel_helpers::Predicate<#table::table>;
        impl ::diesel_aip_160::compiler::Aip160FilterCompiler for #compiler {
            type Predicate = #predicate;
            fn compare(&self, field: &str, operator: ::diesel_aip_160::compiler::ComparisonOperator,
                value: ::diesel_aip_160::compiler::FilterLiteral) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                match (field, value) {
                    #(#comparisons)*
                    _ => ::diesel_aip_160::anyhow::bail!("unknown or unsupported filter field `{field}`"),
                }
            }
            fn constant(&self, value: bool) -> Self::Predicate {
                Box::new(value.into_sql::<::diesel_aip_160::diesel::sql_types::Bool>())
            }
            fn and(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate {
                Box::new(left.and(right))
            }
            fn or(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate {
                Box::new(left.or(right))
            }
            fn not(&self, predicate: Self::Predicate) -> Self::Predicate {
                Box::new(::diesel_aip_160::diesel::dsl::not(predicate))
            }
            fn field_type(&self, field: &str) -> ::diesel_aip_160::anyhow::Result<::diesel_aip_160::compiler::FieldType> {
                #(#json_types)*
                match field { #(#types)* _ => ::diesel_aip_160::anyhow::bail!("unknown or unsupported filter field `{field}"), }
            }
            fn text_fields(&self) -> &'static [&'static str] { &[#(#text_names),*] }
            fn like(&self, field: &str, pattern: String) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                match field { #(#likes)* _ => ::diesel_aip_160::anyhow::bail!("unknown or unsupported string field `{field}"), }
            }
            fn json_compare(&self, field: &str, operator: ::diesel_aip_160::compiler::ComparisonOperator, value: ::diesel_aip_160::serde_json::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                #(#json_comparisons)*
                ::diesel_aip_160::anyhow::bail!("unknown JSONB field `{field}`")
            }
            fn json_contains(&self, field: &str, value: ::diesel_aip_160::serde_json::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                #(#json_containments)*
                ::diesel_aip_160::anyhow::bail!("unknown JSONB field `{field}`")
            }
            fn json_present(&self, field: &str) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                #(#json_presences)*
                ::diesel_aip_160::anyhow::bail!("unknown JSONB field `{field}`")
            }
        }
    })
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

fn is_type(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == name))
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
        println!("Generated code: {}", output);
        assert!(output.contains("RecordFilterCompiler"));
        assert!(output.contains("FieldType::String{nullable:false}"));
        assert!(output.contains("FieldType::String{nullable:true}"));
        assert!(output.contains("FieldType::Int32"));
        assert!(output.contains("json_path::<Details>(field,\"details\")"));
        assert!(!output.contains("\"ignored\""));
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

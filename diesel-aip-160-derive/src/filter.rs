//! Model filter compiler generation.

use quote::{format_ident, quote};
use syn::{DeriveInput, GenericArgument, PathArguments, Type};

use crate::attributes::{named_fields, options};

pub(super) fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut table = None;
    for attribute in &input.attrs {
        if attribute.path().is_ident("diesel") {
            for option in options(attribute)? {
                if option.path().is_ident("table_name") {
                    let syn::Meta::NameValue(option) = option else {
                        return Err(syn::Error::new_spanned(
                            option,
                            "expected table_name = path::table",
                        ));
                    };
                    if table.is_some() {
                        return Err(syn::Error::new_spanned(option, "duplicate table_name"));
                    }
                    let value = option.value;
                    table = Some(syn::parse2::<syn::Path>(quote!(#value))?);
                }
            }
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
    let fields = named_fields(&input, "Aip160Filter")?;
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

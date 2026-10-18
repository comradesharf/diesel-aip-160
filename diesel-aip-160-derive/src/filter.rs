//! Model filter compiler generation.

use quote::{format_ident, quote};
use syn::{DeriveInput, GenericArgument, PathArguments, Type};

use crate::attributes::{named_fields, options};

mod field_options;
use field_options::{FieldOptions, Search, field_options};

pub(super) fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut global_search = None;
    for attribute in &input.attrs {
        if attribute.path().is_ident("aip160") {
            attribute.parse_nested_meta(|meta| {
                if !meta.path.is_ident("global_search_adapter") {
                    return Err(
                        meta.error("expected global_search_adapter = path::Adapter on a model")
                    );
                }
                if global_search.is_some() {
                    return Err(meta.error("duplicate global_search_adapter"));
                }
                global_search = Some(meta.value()?.parse::<syn::Path>()?);
                Ok(())
            })?;
        }
    }
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
    let mut searches = Vec::new();
    let mut query_searches = Vec::new();
    let mut query_customs = Vec::new();
    let mut postgres_search = false;
    for field in &fields.named {
        let FieldOptions {
            skip,
            adapter,
            search_adapter,
            search,
        } = field_options(field)?;
        if skip {
            continue;
        }
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let (ty, nullable) = unwrap_option(&field.ty);
        let column = quote!(#table::#ident);
        if let Some(search_adapter) = search_adapter {
            query_searches.push(quote!(#name => <#search_adapter as ::diesel_aip_160::diesel_helpers::SearchAdapter<#table::table>>::search_query(value),));
            searches.push(quote!(#name => <#search_adapter as ::diesel_aip_160::diesel_helpers::SearchAdapter<#table::table>>::search(value),));
            // Explicit search adapters opt custom fields into global expansion too.
            texts.push(quote!(fields.push(#name);));
        } else if let Some(Search {
            configuration,
            vector,
        }) = search
        {
            if !matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "String"))
            {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "built-in full-text search requires String or Option<String>; use search_adapter for other types",
                ));
            }
            postgres_search = true;
            let (target, function) = if let Some(vector) = vector {
                (quote!(#vector), format_ident!("vector"))
            } else {
                (column.clone(), format_ident!("text"))
            };
            let query_function = format_ident!("{}_query", function);
            searches.push(quote!(#name => ::diesel_aip_160::search::postgres::#function(#target, #configuration, value),));
            query_searches.push(quote!(#name => ::diesel_aip_160::search::postgres::#query_function(#target, #configuration, value),));
            texts.push(quote!(fields.push(#name);));
        } else if adapter.is_none() {
            texts.push(quote!(if <#ty>::__aip160_is_text(#column) { fields.push(#name); }));
        }
        if let Some(adapter) = adapter {
            types.push(quote!(if field == #name { return Ok(::diesel_aip_160::compiler::FieldType::Custom { nullable: #nullable }); }));
            query_customs.push(quote!(#name => <#adapter as ::diesel_aip_160::diesel_helpers::FieldAdapter<#column>>::compare_query(#column, operator, value),));
            customs.push(quote!(#name => <#adapter as ::diesel_aip_160::diesel_helpers::FieldAdapter<#column>>::compare(#column, operator, value),));
            continue;
        }
        types.push(quote!(if let Some(ty) = <#ty>::__aip160_field_type(#column, field, #name) { return Ok(ty); }));
        natives.push(
            quote!(if <#ty>::__aip160_field_type(#column, field, #name).is_some() {
                return <#ty>::__aip160_compare(#column, field, #name, operator, value);
            }),
        );
        ilikes.push(quote!(#name => <#ty>::__aip160_like(#column, pattern, true),));
    }
    let query_global_search = if let Some(adapter) = &global_search {
        quote!(<#adapter as ::diesel_aip_160::diesel_helpers::SearchAdapter<#table::table>>::search_query(value))
    } else {
        quote!(::diesel_aip_160::compiler::query_global_fields(self, value))
    };
    let global_search = global_search.map(|adapter| quote! {
        fn global_search(&self, value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
            <#adapter as ::diesel_aip_160::diesel_helpers::SearchAdapter<#table::table>>::search(value)
        }
    });
    let backend_check =
        postgres_search.then(|| quote!(::diesel_aip_160::__aip160_require_postgres_search!();));
    Ok(quote! {
        #backend_check
        impl #model {
            pub fn compile_query_filter(source: &str) -> ::diesel_aip_160::anyhow::Result<Option<::diesel_aip_160::ranking::CompiledQueryFilter<#table::table>>> {
                ::diesel_aip_160::compiler::compile_query(source, &#compiler)
            }

            pub fn compile_filter(source: &str) -> ::diesel_aip_160::anyhow::Result<Option<::diesel_aip_160::diesel_helpers::Predicate<#table::table>>> {
                ::diesel_aip_160::compiler::compile(source, &#compiler)
            }
        }
        pub struct #compiler;
        impl ::diesel_aip_160::compiler::QueryFilterCompiler for #compiler {
            type Signal = ::diesel_aip_160::ranking::RankingSignal<#table::table>;
            fn query_field_search(&self, field: &str, value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<::diesel_aip_160::ranking::CompiledQueryFilter<#table::table>> {
                match field {
                    #(#query_searches)*
                    _ => ::diesel_aip_160::compiler::Aip160FilterCompiler::field_search(self, field, value).map(::diesel_aip_160::compiler::QueryFilter::predicate),
                }
            }
            fn query_global_search(&self, value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<::diesel_aip_160::ranking::CompiledQueryFilter<#table::table>> {
                #query_global_search
            }
            fn query_custom_compare(&self, field: &str, operator: ::diesel_aip_160::compiler::ComparisonOperator, value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<::diesel_aip_160::ranking::CompiledQueryFilter<#table::table>> {
                match field { #(#query_customs)* _ => ::diesel_aip_160::anyhow::bail!("unknown adapter field `{field}`"), }
            }
            fn finish_signals(&self, signals: Vec<Self::Signal>) -> ::diesel_aip_160::anyhow::Result<Vec<Self::Signal>> {
                Ok(::diesel_aip_160::search::finish_signals(signals))
            }
        }

        impl ::diesel_aip_160::compiler::Aip160FilterCompiler for #compiler {
            type Predicate = ::diesel_aip_160::diesel_helpers::Predicate<#table::table>;
            #global_search
            fn field_search(&self, field: &str, value: &::diesel_aip_160::parser::Value) -> ::diesel_aip_160::anyhow::Result<Self::Predicate> {
                match field {
                    #(#searches)*
                    _ => self.ilike(field, ::diesel_aip_160::compiler::search_pattern(&value.text)),
                }
            }
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

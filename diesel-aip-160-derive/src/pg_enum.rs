use quote::quote;
use syn::{Data, DeriveInput, Fields, Type};

pub(super) fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let inner = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0].ty,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "Aip160PgEnum requires a single-field tuple struct",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Aip160PgEnum requires a single-field tuple struct",
            ));
        }
    };
    let mut sql_type: Option<Type> = None;
    let mut value_type: Option<Type> = None;
    let mut from_value: Option<syn::Path> = None;
    let mut to_value: Option<syn::Path> = None;
    for attribute in &input.attrs {
        if attribute.path().is_ident("diesel") {
            // Other Diesel derives can share this attribute (e.g. not_sized).
            let entries = attribute.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )?;
            for entry in entries {
                if entry.path().is_ident("sql_type") {
                    let syn::Meta::NameValue(entry) = entry else {
                        return Err(syn::Error::new_spanned(
                            entry,
                            "expected sql_type = path::SqlType",
                        ));
                    };
                    if sql_type.is_some() {
                        return Err(syn::Error::new_spanned(entry, "duplicate sql_type"));
                    }
                    let value = entry.value;
                    sql_type = Some(syn::parse2(quote!(#value))?);
                }
            }
        } else if attribute.path().is_ident("aip160") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("value_type") {
                    if value_type.is_some() {
                        return Err(meta.error("duplicate value_type"));
                    }
                    value_type = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("from_value") {
                    if from_value.is_some() {
                        return Err(meta.error("duplicate from_value"));
                    }
                    from_value = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("to_value") {
                    if to_value.is_some() {
                        return Err(meta.error("duplicate to_value"));
                    }
                    to_value = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error("expected value_type, from_value, or to_value"));
                }
                Ok(())
            })?;
        }
    }
    let required =
        |name: &str| syn::Error::new_spanned(&input.ident, format!("Aip160PgEnum requires {name}"));
    let sql_type = sql_type.ok_or_else(|| required("#[diesel(sql_type = ...)]"))?;
    let value_type = value_type.ok_or_else(|| required("#[aip160(value_type = ...)]"))?;
    let from_value = from_value.ok_or_else(|| required("#[aip160(from_value = ...)]"))?;
    let to_value = to_value.ok_or_else(|| required("#[aip160(to_value = ...)]"))?;
    let model = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::diesel_aip_160::diesel::deserialize::FromSql<
            #sql_type, ::diesel_aip_160::diesel::pg::Pg,
        > for #model #type_generics #where_clause {
            fn from_sql(bytes: ::diesel_aip_160::diesel::pg::PgValue<'_>)
                -> ::diesel_aip_160::diesel::deserialize::Result<Self>
            {
                let value = ::std::str::from_utf8(bytes.as_bytes())?.parse::<#value_type>()?;
                let inner: #inner = #from_value(value)?;
                Ok(Self(inner))
            }
        }

        impl #impl_generics ::diesel_aip_160::diesel::serialize::ToSql<
            #sql_type, ::diesel_aip_160::diesel::pg::Pg,
        > for #model #type_generics #where_clause {
            fn to_sql<'__aip160_bind>(
                &'__aip160_bind self,
                out: &mut ::diesel_aip_160::diesel::serialize::Output<
                    '__aip160_bind, '_, ::diesel_aip_160::diesel::pg::Pg,
                >,
            ) -> ::diesel_aip_160::diesel::serialize::Result {
                let value: #value_type = #to_value(&self.0)?;
                ::std::io::Write::write_fmt(out, ::std::format_args!("{}", value))?;
                Ok(::diesel_aip_160::diesel::serialize::IsNull::No)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_shapes_and_options() {
        for input in [
            syn::parse_quote!(
                enum Wrapper {
                    A,
                }
            ),
            syn::parse_quote!(
                struct Wrapper;
            ),
            syn::parse_quote!(
                struct Wrapper {
                    value: i16,
                }
            ),
            syn::parse_quote!(
                struct Wrapper(i16, i16);
            ),
        ] {
            assert!(
                expand(input)
                    .unwrap_err()
                    .to_string()
                    .contains("single-field tuple struct")
            );
        }
        for options in [
            quote!(value_type = i16, value_type = i32),
            quote!(from_value = a, from_value = b),
            quote!(to_value = a, to_value = b),
            quote!(unknown = a),
        ] {
            let input = syn::parse_quote! {
                #[diesel(sql_type = SqlType)]
                #[aip160(#options)]
                struct Wrapper(External);
            };
            assert!(expand(input).is_err());
        }
    }

    #[test]
    fn requires_each_codec_option() {
        for (attrs, missing) in [
            (quote!(), "sql_type"),
            (quote!(#[diesel(sql_type = SqlType)]), "value_type"),
            (
                quote!(#[diesel(sql_type = SqlType)] #[aip160(value_type = i16)]),
                "from_value",
            ),
            (
                quote!(#[diesel(sql_type = SqlType)] #[aip160(value_type = i16, from_value = decode)]),
                "to_value",
            ),
        ] {
            let input = syn::parse_quote! { #attrs struct Wrapper(External); };
            assert!(expand(input).unwrap_err().to_string().contains(missing));
        }
    }
}

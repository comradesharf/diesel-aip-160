//! JSONB field-name mapping generation.

use quote::quote;
use syn::DeriveInput;

use crate::attributes::{named_fields, options};

pub(super) fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let fields = named_fields(&input, "Aip160Jsonb")?;
    for attribute in &input.attrs {
        if attribute.path().is_ident("serde") {
            for option in options(attribute)? {
                if option.path().is_ident("rename_all") {
                    return Err(syn::Error::new_spanned(
                        option,
                        "Aip160Jsonb does not support serde rename_all",
                    ));
                }
            }
        }
    }
    let mut mappings = Vec::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let mut stored = name.clone();
        for attribute in &field.attrs {
            if attribute.path().is_ident("serde") {
                for option in options(attribute)? {
                    if option.path().is_ident("rename") {
                        let syn::Meta::NameValue(option) = option else {
                            return Err(syn::Error::new_spanned(
                                option,
                                "expected rename = \"stored_key\"",
                            ));
                        };
                        let value = option.value;
                        stored = syn::parse2::<syn::LitStr>(quote!(#value))?.value();
                    }
                }
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

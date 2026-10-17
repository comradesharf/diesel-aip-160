//! Shared validation and parsing for attributes owned by other derives.

use syn::{Attribute, Data, DeriveInput, Fields, FieldsNamed, Meta, Token, punctuated::Punctuated};

pub(super) fn named_fields<'a>(
    input: &'a DeriveInput,
    derive: &str,
) -> syn::Result<&'a FieldsNamed> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            format!("{derive} requires a struct"),
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            format!("{derive} requires named fields"),
        ));
    };
    Ok(fields)
}

// Parse each option separately so ignoring an unrelated option cannot consume
// a later option in the same attribute.
pub(super) fn options(attribute: &Attribute) -> syn::Result<Punctuated<Meta, Token![,]>> {
    attribute.parse_args_with(Punctuated::parse_terminated)
}

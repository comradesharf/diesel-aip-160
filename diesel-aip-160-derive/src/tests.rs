use super::*;
use quote::quote;

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
    let output = compact(jsonb::expand(input).unwrap());
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
        jsonb::expand(renamed)
            .unwrap_err()
            .to_string()
            .contains("rename_all")
    );

    let tuple = syn::parse_quote! { struct Details(String); };
    assert!(
        jsonb::expand(tuple)
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
    let output = compact(filter::expand(input).unwrap());
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
    let output = compact(filter::expand(input).unwrap());
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
    assert!(filter::expand(input).is_ok());
    for attributes in [
        quote!(skip, adapter = Adapter),
        quote!(adapter = A, adapter = B),
        quote!(unknown),
    ] {
        let input = syn::parse_quote! {
            #[diesel(table_name = records)]
            struct Record { #[aip160(#attributes)] code: Code }
        };
        assert!(filter::expand(input).is_err());
    }
}

#[test]
fn filter_requires_a_table_name() {
    let input = syn::parse_quote! { struct Record { name: String } };
    assert!(
        filter::expand(input)
            .unwrap_err()
            .to_string()
            .contains("table_name")
    );
}

#[test]
fn unrelated_diesel_options_do_not_hide_the_table_name() {
    for attributes in [
        quote!(check_for_backend(diesel::pg::Pg), table_name = records),
        quote!(table_name = records, check_for_backend(diesel::pg::Pg)),
    ] {
        let input = syn::parse_quote! {
            #[diesel(#attributes)]
            struct Record { name: String }
        };
        let output = compact(filter::expand(input).unwrap());
        assert!(output.contains("records::name"));
    }
}

#[test]
fn duplicate_table_names_are_rejected() {
    let input = syn::parse_quote! {
        #[diesel(table_name = records, table_name = other)]
        struct Record { name: String }
    };
    assert!(
        filter::expand(input)
            .unwrap_err()
            .to_string()
            .contains("duplicate table_name")
    );
}

#[test]
fn unrelated_serde_options_do_not_hide_renames() {
    let input = syn::parse_quote! {
        struct Details {
            #[serde(default, rename = "stored_name", skip_serializing_if = "Option::is_none")]
            field: Option<String>,
        }
    };
    let output = compact(jsonb::expand(input).unwrap());
    assert!(output.contains("\"field\"=>Some(\"stored_name\")"));

    let input = syn::parse_quote! {
        #[serde(default, rename_all = "camelCase")]
        struct Details { field_name: String }
    };
    assert!(
        jsonb::expand(input)
            .unwrap_err()
            .to_string()
            .contains("rename_all")
    );
}

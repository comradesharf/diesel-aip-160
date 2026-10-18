//! Field search and adapter attribute validation.

pub(super) struct Search {
    pub(super) configuration: syn::LitStr,
    pub(super) vector: Option<syn::Path>,
}

#[derive(Default)]
pub(super) struct FieldOptions {
    pub(super) skip: bool,
    pub(super) adapter: Option<syn::Path>,
    pub(super) search_adapter: Option<syn::Path>,
    pub(super) search: Option<Search>,
}

pub(super) fn field_options(field: &syn::Field) -> syn::Result<FieldOptions> {
    let mut result = FieldOptions::default();
    for attribute in &field.attrs {
        if attribute.path().is_ident("aip160") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    if result.skip { return Err(meta.error("duplicate skip")); }
                    result.skip = true;
                } else if meta.path.is_ident("adapter") {
                    if result.adapter.is_some() { return Err(meta.error("duplicate adapter")); }
                    result.adapter = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("search_adapter") {
                    if result.search_adapter.is_some() { return Err(meta.error("duplicate search_adapter")); }
                    result.search_adapter = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("search") {
                    if result.search.is_some() { return Err(meta.error("duplicate search")); }
                    let mut postgres = false;
                    let mut configuration = None;
                    let mut vector = None;
                    meta.parse_nested_meta(|option| {
                        if option.path.is_ident("postgres") {
                            if postgres { return Err(option.error("duplicate postgres")); }
                            postgres = true;
                        } else if option.path.is_ident("configuration") {
                            if configuration.is_some() { return Err(option.error("duplicate configuration")); }
                            let name: syn::LitStr = option.value()?.parse()?;
                            if name.value().is_empty() || !name.value().bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.') {
                                return Err(option.error("configuration must be an optionally schema-qualified ASCII name"));
                            }
                            configuration = Some(name);
                        } else if option.path.is_ident("vector") {
                            if vector.is_some() { return Err(option.error("duplicate vector")); }
                            vector = Some(option.value()?.parse()?);
                        } else {
                            return Err(option.error("built-in search supports only postgres, configuration = \"english\", and optional vector = path::column; use search_adapter for MySQL/SQLite"));
                        }
                        Ok(())
                    })?;
                    if !postgres { return Err(meta.error("built-in search requires postgres; MySQL/SQLite require search_adapter")); }
                    let configuration = configuration.ok_or_else(|| meta.error("search requires an explicit configuration = \"english\""))?;
                    result.search = Some(Search { configuration, vector });
                } else {
                    return Err(meta.error("expected skip, adapter, search_adapter, or search(postgres, configuration = \"english\")"));
                }
                Ok(())
            })?;
        }
    }
    if result.skip
        && (result.adapter.is_some() || result.search_adapter.is_some() || result.search.is_some())
    {
        return Err(syn::Error::new_spanned(
            field,
            "skip cannot be combined with adapter or search configuration",
        ));
    }
    if result.search.is_some() && (result.search_adapter.is_some() || result.adapter.is_some()) {
        return Err(syn::Error::new_spanned(
            field,
            "built-in search cannot be combined with search_adapter or adapter; combine adapter and search_adapter for custom types",
        ));
    }
    Ok(result)
}

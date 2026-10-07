// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Expansion of `#[derive(Model)]`: cross-field validation, then codegen.

use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Fields, LitStr};

use crate::parse::{Column, parse_fields, parse_struct_attrs};
use crate::types;

pub(crate) fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let named = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            Fields::Unnamed(fields) => {
                return Err(Error::new(
                    fields.span(),
                    "bee_orm: Model requires a struct with named fields",
                ));
            }
            Fields::Unit => {
                return Err(Error::new(
                    struct_name.span(),
                    "bee_orm: Model requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(Error::new(
                struct_name.span(),
                "bee_orm: Model can only be derived for a struct",
            ));
        }
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new(
            input.generics.span(),
            "bee_orm: Model does not support generic structs",
        ));
    }

    let mut errors: Vec<Error> = Vec::new();

    let (table, orm, hooks, m2m) = parse_struct_attrs(&input, &mut errors);
    // §61.2: every emitted path goes through `#orm`; without the attribute it
    // is the literal `bee_orm` path, so the expansion is byte-identical to
    // what pre-attribute versions produced.
    let orm: syn::Path = orm.unwrap_or_else(|| syn::parse_quote!(bee_orm));
    let columns = parse_fields(named, &mut errors);

    // The primary key is the `#[bee(pk)]` field, or else the non-ignored field
    // literally named `id`.
    let explicit_pk: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.pk)
        .map(|(index, _)| index)
        .collect();
    let pk_index = if explicit_pk.len() > 1 {
        let names: Vec<String> =
            explicit_pk.iter().map(|&index| columns[index].ident.to_string()).collect();
        errors.push(Error::new(
            columns[explicit_pk[1]].ident.span(),
            format!("bee_orm: multiple #[bee(pk)] fields (`{}`)", names.join("`, `")),
        ));
        None
    } else if explicit_pk.len() == 1 {
        Some(explicit_pk[0])
    } else {
        let id = columns.iter().position(|column| !column.ignore && column.ident.unraw() == "id");
        if id.is_none() {
            errors.push(Error::new(
                struct_name.span(),
                "bee_orm: no primary key: mark a field #[bee(pk)] or name it `id`",
            ));
        }
        id
    };

    // Only checked once a pk resolved: with none (or several) the error above
    // already names the root cause, and "auto is not on the pk" would repeat it.
    if let Some(pk_position) = pk_index {
        for (index, column) in columns.iter().enumerate() {
            if column.auto && index != pk_position {
                errors.push(Error::new(
                    column.ident.span(),
                    "bee_orm: #[bee(auto)] is only valid on the primary key field",
                ));
            }
        }
        if columns[pk_position].soft_delete {
            errors.push(Error::new(
                columns[pk_position].ident.span(),
                "bee_orm: #[bee(soft_delete)] is not valid on the primary key field",
            ));
        }
        if columns[pk_position].fk.is_some() {
            errors.push(Error::new(
                columns[pk_position].ident.span(),
                "bee_orm: #[bee(fk)] is not valid on the primary key field",
            ));
        }
    }

    let soft_deletes: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.soft_delete)
        .map(|(index, _)| index)
        .collect();
    if soft_deletes.len() > 1 {
        errors.push(Error::new(
            columns[soft_deletes[1]].ident.span(),
            "bee_orm: multiple #[bee(soft_delete)] fields",
        ));
    }

    let mut seen: Vec<String> = Vec::new();
    for column in &columns {
        if column.ignore {
            continue;
        }
        let name = column.column.value();
        if seen.contains(&name) {
            errors.push(Error::new(
                column.column.span(),
                format!("bee_orm: duplicate column name `{name}`"),
            ));
        } else {
            seen.push(name);
        }
    }

    // §34: metadata for `columns()`, declaration order, ignored fields
    // excluded. Timestamps force `BigInt` / NOT NULL / `DEFAULT 0`, the
    // soft-delete flag forces NOT NULL / `DEFAULT FALSE`; a `sql_type` override
    // wins the type string in both cases. Unmapped spellings need the override
    // (§37.2); `auto` additionally requires an integer pk.
    let mut column_defs: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut fk_assertions: Vec<proc_macro2::TokenStream> = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if column.ignore {
            continue;
        }
        let timestamp = column.auto_now_add || column.auto_now;
        let (option_spelled, inner) = types::strip_option(&column.ty);
        let mapped = types::spelling(inner);
        let sql = match &column.sql_type {
            Some(literal) => quote! { #orm::model::SqlType::Raw(#literal) },
            None if timestamp => quote! { #orm::model::SqlType::BigInt },
            None => match mapped {
                Some(variant) => {
                    let variant = syn::Ident::new(variant, proc_macro2::Span::call_site());
                    quote! { #orm::model::SqlType::#variant }
                }
                None => {
                    // Whitespace-normalised spelling, as written in the source.
                    let spelling = quote!(#inner).to_string().replace(' ', "");
                    // §45: a `Value` spelling gets the JSON-specific hint.
                    let hint = if types::last_segment_is_value(inner) {
                        format!(
                            "bee_orm: no SQL type mapping for `{spelling}`: use \
                             `serde_json::Value` or add #[bee(sql_type = \"...\")] to override"
                        )
                    } else {
                        format!(
                            "bee_orm: no SQL type mapping for `{spelling}`: add \
                             #[bee(sql_type = \"...\")] to override"
                        )
                    };
                    errors.push(Error::new(column.ty.span(), hint));
                    continue;
                }
            },
        };

        let is_pk = Some(index) == pk_index;
        let auto_increment = is_pk && column.auto;
        // Unmapped spellings already error above; only a *mapped* non-integer
        // pk needs this second diagnostic.
        if auto_increment && column.sql_type.is_none() && !matches!(mapped, Some("Int" | "BigInt"))
        {
            errors.push(Error::new(
                column.ident.span(),
                "bee_orm: #[bee(auto)] requires an integer primary key: add \
                 #[bee(sql_type = \"...\")] to override",
            ));
        }

        let nullable = option_spelled && !timestamp && !column.soft_delete;
        let default = if timestamp {
            quote! { Some(#orm::model::DefaultValue::Int(0)) }
        } else if column.soft_delete {
            quote! { Some(#orm::model::DefaultValue::Bool(false)) }
        } else {
            quote! { None }
        };
        let references = match &column.fk {
            Some(target) => {
                // Spanned at the `fk` target so a non-`Model` target reports
                // there (§37.3); the qualified paths alone would be cryptic.
                fk_assertions.push(quote_spanned! { target.span() =>
                    const _: fn() = || {
                        fn assert_model<T: #orm::Model>() {}
                        assert_model::<#target>();
                    };
                });
                quote_spanned! { target.span() =>
                    Some(#orm::model::Reference {
                        table: <#target as #orm::Model>::table_name,
                        pk_column: <#target as #orm::Model>::pk_column,
                    })
                }
            }
            None => quote! { None },
        };

        let name = &column.column;
        column_defs.push(quote! {
            #orm::model::ColumnDef {
                name: #name,
                sql: #sql,
                nullable: #nullable,
                primary_key: #is_pk,
                auto_increment: #auto_increment,
                default: #default,
                references: #references,
            }
        });
    }

    // §43: join-table metadata, omit-when-unused — a model without any `m2m`
    // attribute expands byte-identically to round 4. The slice promotes to
    // `&'static` like `columns()` does, fn pointers included.
    let m2m_defs: Vec<proc_macro2::TokenStream> = m2m
        .iter()
        .map(|def| {
            let target = &def.target;
            let (table, local, foreign) = (&def.table, &def.local, &def.foreign);
            let target_ident = &def.target_ident;
            quote_spanned! { target.span() =>
                #orm::model::M2mDef {
                    table: #table,
                    local_column: #local,
                    foreign_column: #foreign,
                    target_ident: #target_ident,
                    target_table: <#target as #orm::Model>::table_name,
                    target_columns: <#target as #orm::Model>::columns,
                }
            }
        })
        .collect();
    let m2m_method = if m2m_defs.is_empty() {
        None
    } else {
        Some(quote! {
            fn m2m() -> &'static [#orm::model::M2mDef] {
                &[#(#m2m_defs),*]
            }
        })
    };
    let m2m_assertions: Vec<proc_macro2::TokenStream> = m2m
        .iter()
        .map(|def| {
            let span = def.target.span();
            // The const sits outside the impl, where `Self` is not a type —
            // the asserting spelling of a `Self` target is the struct name.
            let target: proc_macro2::TokenStream = if def.target.is_ident("Self") {
                quote!(#struct_name)
            } else {
                let target = &def.target;
                quote!(#target)
            };
            quote_spanned! { span =>
                const _: fn() = || {
                    fn assert_model<T: #orm::Model>() {}
                    assert_model::<#target>();
                };
            }
        })
        .collect();

    let mut errors = errors.into_iter();
    if let Some(mut first) = errors.next() {
        for error in errors {
            first.combine(error);
        }
        return Err(first);
    }

    let pk_index = match pk_index {
        Some(index) => index,
        None => {
            return Err(Error::new(
                struct_name.span(),
                "bee_orm: no primary key: mark a field #[bee(pk)] or name it `id`",
            ));
        }
    };
    let pk = &columns[pk_index];

    let table = match table {
        Some(literal) => literal,
        None => {
            LitStr::new(&(struct_name.unraw().to_string().to_lowercase() + "s"), struct_name.span())
        }
    };

    // Field declaration order everywhere; `insert_values` skips `auto`,
    // `ignore` and the timestamps; `update_values` skips the primary key,
    // `ignore` and the timestamps. Timestamps are injected by the trait (one
    // value per statement), so the lists must not carry them either — a
    // duplicate column would break PostgreSQL's `SET`. The soft-delete flag is
    // an ordinary writable column and stays in both lists.
    let from_row = columns.iter().map(|column| {
        let ident = &column.ident;
        if column.ignore {
            quote! { #ident: ::core::default::Default::default() }
        } else {
            let name = &column.column;
            quote! { #ident: #orm::decode(row, #name)? }
        }
    });
    let insert_values = columns
        .iter()
        .filter(|column| !column.auto && !column.ignore && !column.auto_now_add && !column.auto_now)
        .map(|column| {
            let ident = &column.ident;
            let name = &column.column;
            quote! { (#name, #orm::Value::from(self.#ident.clone())) }
        });
    let update_values = columns
        .iter()
        .enumerate()
        .filter(|(index, column)| {
            *index != pk_index && !column.ignore && !column.auto_now_add && !column.auto_now
        })
        .map(|(_, column)| {
            let ident = &column.ident;
            let name = &column.column;
            quote! { (#name, #orm::Value::from(self.#ident.clone())) }
        });

    // Emitted only when the corresponding attribute is used, so a round-2
    // struct expands exactly as before.
    let column_list = |name: &str,
                       select: fn(&Column) -> bool|
     -> Option<proc_macro2::TokenStream> {
        let names: Vec<&LitStr> =
            columns.iter().filter(|column| select(column)).map(|column| &column.column).collect();
        if names.is_empty() {
            return None;
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        Some(quote! {
            fn #method() -> &'static [&'static str] {
                &[#(#names),*]
            }
        })
    };
    let auto_now_add_columns = column_list("auto_now_add_columns", |column| column.auto_now_add);
    let auto_now_columns = column_list("auto_now_columns", |column| column.auto_now);
    let soft_delete_column = columns.iter().find(|column| column.soft_delete).map(|column| {
        let name = &column.column;
        quote! {
            fn soft_delete_column() -> Option<&'static str> {
                Some(#name)
            }
        }
    });

    // One forwarder per listed hook; `Self::hook(self)` resolves to the
    // same-named inherent method (never recurses into the trait default).
    let hook_forwarders = hooks.iter().copied().map(|name| {
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        quote! {
            async fn #method(&self) -> #orm::Result<()> {
                Self::#method(self).await
            }
        }
    });
    // An impl overriding async trait methods must carry the attribute; an impl
    // without hooks stays unannotated.
    let impl_attribute =
        if hooks.is_empty() { None } else { Some(quote! { #[#orm::__private::async_trait] }) };

    let pk_ident = &pk.ident;
    let pk_column = &pk.column;

    Ok(quote! {
        #impl_attribute
        impl #orm::Model for #struct_name {
            fn table_name() -> &'static str {
                #table
            }

            fn pk_column() -> &'static str {
                #pk_column
            }

            fn from_row(row: &#orm::Row) -> #orm::Result<Self> {
                Ok(Self {
                    #(#from_row),*
                })
            }

            fn insert_values(&self) -> ::std::vec::Vec<(&'static str, #orm::Value)> {
                ::std::vec![#(#insert_values),*]
            }

            fn pk_value(&self) -> #orm::Value {
                #orm::Value::from(self.#pk_ident.clone())
            }

            fn update_values(&self) -> ::std::vec::Vec<(&'static str, #orm::Value)> {
                ::std::vec![#(#update_values),*]
            }

            fn columns() -> &'static [#orm::model::ColumnDef] {
                &[#(#column_defs),*]
            }

            #m2m_method

            #auto_now_add_columns
            #auto_now_columns
            #soft_delete_column
            #(#hook_forwarders)*
        }

        #(#fk_assertions)*
        #(#m2m_assertions)*

        impl #struct_name {
            /// The table this model is mapped to.
            pub fn table_name() -> &'static str {
                #table
            }

            /// Build a query set for this model's table.
            pub fn query() -> #orm::QuerySet<Self> {
                #orm::QuerySet::new(#table)
            }
        }
    })
}

// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `#[bee(...)]` parsing: struct-level keys, per-field flags and column
//! resolution. Cross-field checks live in `expand`.

use quote::quote;
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{DeriveInput, Error, Field, LitStr, token};

/// Parsed `#[bee(...)]` flags of one field.
#[derive(Default)]
struct FieldOptions {
    column: Option<LitStr>,
    pk: bool,
    auto: bool,
    ignore: bool,
    auto_now_add: bool,
    auto_now: bool,
    soft_delete: bool,
    sql_type: Option<LitStr>,
    fk: Option<syn::Path>,
}

/// A mapped field after attribute parsing and column resolution.
pub(crate) struct Column {
    pub(crate) ident: syn::Ident,
    pub(crate) ty: syn::Type,
    pub(crate) column: LitStr,
    pub(crate) pk: bool,
    pub(crate) auto: bool,
    pub(crate) ignore: bool,
    pub(crate) auto_now_add: bool,
    pub(crate) auto_now: bool,
    pub(crate) soft_delete: bool,
    pub(crate) sql_type: Option<LitStr>,
    pub(crate) fk: Option<syn::Path>,
}

/// One struct-level `#[bee(m2m(Target, …))]` after §43 resolution: the string
/// literals already carry the convention defaults, so `expand` only quotes
/// them. `target_ident` is §55's verbatim spelling (case kept), a different
/// value from the lowercased defaults below even though both come from the
/// same path.
pub(crate) struct M2m {
    pub(crate) target: syn::Path,
    pub(crate) target_ident: LitStr,
    pub(crate) table: LitStr,
    pub(crate) local: LitStr,
    pub(crate) foreign: LitStr,
}

/// §43: one `m2m(...)` names one target — a repeat is always an authoring bug.
const DUPLICATE_M2M: &str = "bee_orm: duplicate m2m target: each target can appear only once";

/// §61.2 `#[bee(crate = "…")]` value errors: a bad path there would silently
/// mis-point every emitted path, so each shape fails loudly.
const CRATE_EXPECTS_LITERAL: &str = "bee_orm: #[bee(crate = \"path\")] expects a string literal";
const CRATE_EMPTY: &str = "bee_orm: #[bee(crate = \"path\")] must not be empty";
const CRATE_NOT_A_PATH: &str = "bee_orm: #[bee(crate = \"path\")] is not a valid path";

/// Records one `table = "…"` / `local = "…"` / `foreign = "…"` of an `m2m`.
/// Repeating a key is an authoring bug (§18.3), never a last-wins.
fn set_m2m_option(
    slot: &mut Option<LitStr>,
    option: &syn::meta::ParseNestedMeta<'_>,
) -> syn::Result<()> {
    if slot.replace(option.value()?.parse()?).is_some() {
        return Err(option.error("bee_orm: duplicate #[bee(m2m)] option"));
    }
    Ok(())
}

/// The lifecycle hooks the derive can forward, in trait declaration order.
const HOOKS: [&str; 6] = [
    "before_insert",
    "after_insert",
    "before_update",
    "after_update",
    "before_delete",
    "after_delete",
];

/// Whether `name` matches `[A-Za-z_][A-Za-z0-9_]*`.
fn is_valid_column(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The `table` override (if any), the §61.2 `crate` path (the ORM crate the
/// expansion prefixes every path with), the forward-list `hooks` (declaration
/// order, duplicates silently dropped) and the resolved `m2m` targets.
pub(crate) fn parse_struct_attrs(
    input: &DeriveInput,
    errors: &mut Vec<Error>,
) -> (Option<LitStr>, Option<syn::Path>, Vec<&'static str>, Vec<M2m>) {
    let mut table: Option<LitStr> = None;
    let mut orm: Option<syn::Path> = None;
    let mut hooks: Vec<&'static str> = Vec::new();
    let mut m2m_defs: Vec<M2m> = Vec::new();
    let mut seen_m2m: Vec<String> = Vec::new();
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("bee")) {
        let parsed = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let literal: LitStr = meta.value()?.parse()?;
                // §18.3: duplicates are always authoring bugs, never last-wins.
                if table.replace(literal).is_some() {
                    errors.push(meta.error("bee_orm: duplicate #[bee(table)] attribute"));
                }
                Ok(())
            } else if meta.path.is_ident("hooks") {
                meta.parse_nested_meta(|hook| {
                    let name = match hook.path.get_ident() {
                        Some(ident) => ident.to_string(),
                        None => {
                            let path = &hook.path;
                            quote!(#path).to_string()
                        }
                    };
                    match HOOKS.iter().copied().find(|known| *known == name.as_str()) {
                        // Duplicates are idempotent config: deduplicated silently.
                        Some(known) => {
                            if !hooks.contains(&known) {
                                hooks.push(known);
                            }
                            Ok(())
                        }
                        None => Err(hook.error(format!(
                            "bee_orm: unknown hook `{name}` (expected before_insert, after_insert, \
                             before_update, after_update, before_delete, after_delete)"
                        ))),
                    }
                })
            } else if meta.path.is_ident("crate") {
                // §61.2: the path to the ORM crate, replacing the default
                // `bee_orm` — for users who reach it through a re-export
                // (`bee_rust::bee_orm`) and never depend on it directly.
                // `crate` is a keyword, but `parse_nested_meta` reads the
                // path with `parse_any`, so the segment is already here.
                let literal: LitStr = meta.value()?.parse().map_err(|error| {
                    Error::new(error.span(), CRATE_EXPECTS_LITERAL)
                })?;
                if literal.value().is_empty() {
                    return Err(Error::new(literal.span(), CRATE_EMPTY));
                }
                let path: syn::Path =
                    literal.parse().map_err(|_| Error::new(literal.span(), CRATE_NOT_A_PATH))?;
                if orm.replace(path).is_some() {
                    return Err(meta.error("bee_orm: duplicate #[bee(crate)] attribute"));
                }
                Ok(())
            } else if meta.path.is_ident("m2m") {
                let mut target: Option<syn::Path> = None;
                let mut table: Option<LitStr> = None;
                let mut local: Option<LitStr> = None;
                let mut foreign: Option<LitStr> = None;
                let nested = meta.parse_nested_meta(|option| {
                    if option.path.is_ident("table") {
                        set_m2m_option(&mut table, &option)?;
                    } else if option.path.is_ident("local") {
                        set_m2m_option(&mut local, &option)?;
                    } else if option.path.is_ident("foreign") {
                        set_m2m_option(&mut foreign, &option)?;
                    } else if option.input.is_empty() || option.input.peek(token::Comma) {
                        // A bare type path — the first is the target, a second
                        // bare path in the same list repeats it.
                        if target.is_some() {
                            return Err(option.error(DUPLICATE_M2M));
                        }
                        target = Some(option.path.clone());
                    } else {
                        return Err(option.error(
                            "bee_orm: unknown m2m option (expected table, local, foreign)",
                        ));
                    }
                    Ok(())
                });
                if let Err(error) = nested {
                    // A malformed `m2m(...)` contributes no metadata: report
                    // the parse error alone, never a cascade from a half-read
                    // attribute.
                    errors.push(error);
                } else if let Some(target) = target {
                    let key = quote!(#target).to_string();
                    if seen_m2m.contains(&key) {
                        errors.push(Error::new(target.span(), DUPLICATE_M2M));
                    } else {
                        seen_m2m.push(key);
                    }
                    // §55: the target's verbatim spelling — last segment,
                    // `unraw`, case kept (`Tag` / `crate::models::Tag` →
                    // "Tag", `r#type` → "type"); `Self` is the declaring
                    // struct. §43's default names are its lowercased
                    // derivative, so both spellings share this one source.
                    let verbatim = if target.is_ident("Self") {
                        input.ident.unraw().to_string()
                    } else {
                        target
                            .segments
                            .last()
                            .map(|segment| segment.ident.unraw().to_string())
                            .unwrap_or_default()
                    };
                    // §43: defaults are ident-based (lowercased, no snake_case
                    // pass) so they stay const and survive `#[bee(table = …)]`
                    // renames. `Self` names the declaring model itself.
                    let local_ident = input.ident.unraw().to_string().to_lowercase();
                    let target_ident = verbatim.to_lowercase();
                    let table = table.unwrap_or_else(|| {
                        LitStr::new(&format!("{local_ident}_{target_ident}"), target.span())
                    });
                    let local = local.unwrap_or_else(|| {
                        LitStr::new(&format!("{local_ident}_id"), target.span())
                    });
                    let foreign = foreign.unwrap_or_else(|| {
                        LitStr::new(&format!("{target_ident}_id"), target.span())
                    });
                    // The table default is never ambiguous; only equal column
                    // names (self target or same-ident collision) are an error.
                    if local.value() == foreign.value() {
                        errors.push(Error::new(
                            target.span(),
                            "bee_orm: m2m columns collide: add explicit local and foreign overrides",
                        ));
                    }
                    let target_ident = LitStr::new(&verbatim, target.span());
                    m2m_defs.push(M2m {
                        target,
                        target_ident,
                        table,
                        local,
                        foreign,
                    });
                } else {
                    // `m2m()` with no target at all is an authoring bug, not a
                    // silent no-op.
                    errors.push(Error::new(
                        meta.path.span(),
                        "bee_orm: m2m requires a target type",
                    ));
                }
                Ok(())
            } else {
                Err(meta.error(
                    "bee_orm: unknown bee attribute for a struct (expected table, hooks, m2m, crate)",
                ))
            }
        });
        if let Err(error) = parsed {
            errors.push(error);
        }
    }
    (table, orm, hooks, m2m_defs)
}

/// Resolve every field into a [`Column`], accumulating the per-field errors.
pub(crate) fn parse_fields(
    fields: &Punctuated<Field, token::Comma>,
    errors: &mut Vec<Error>,
) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    for field in fields {
        let ident = match field.ident.clone() {
            Some(ident) => ident,
            None => {
                errors.push(Error::new(
                    field.span(),
                    "bee_orm: Model requires a struct with named fields",
                ));
                return columns;
            }
        };

        let mut options = FieldOptions::default();
        for attr in field.attrs.iter().filter(|attr| attr.path().is_ident("bee")) {
            let parsed = attr.parse_nested_meta(|meta| {
                let name = meta.path.get_ident().map(syn::Ident::to_string).unwrap_or_default();
                match name.as_str() {
                    "column" => options.column = Some(meta.value()?.parse()?),
                    "pk" => options.pk = true,
                    "auto" => options.auto = true,
                    "ignore" => options.ignore = true,
                    "auto_now_add" => options.auto_now_add = true,
                    "auto_now" => options.auto_now = true,
                    "soft_delete" => options.soft_delete = true,
                    "sql_type" => options.sql_type = Some(meta.value()?.parse()?),
                    "fk" => options.fk = Some(meta.value()?.parse()?),
                    _ => {
                        return Err(meta.error(
                            "bee_orm: unknown bee attribute for a field (expected column, pk, \
                             auto, ignore, auto_now_add, auto_now, soft_delete, sql_type, fk)",
                        ));
                    }
                }
                Ok(())
            });
            if let Err(error) = parsed {
                errors.push(error);
            }
        }

        if options.pk && options.ignore {
            errors.push(Error::new(
                ident.span(),
                "bee_orm: #[bee(pk)] and #[bee(ignore)] on the same field",
            ));
        }

        if options.auto_now_add && options.auto_now {
            errors.push(Error::new(
                ident.span(),
                "bee_orm: #[bee(auto_now_add)] and #[bee(auto_now)] on the same field",
            ));
        }
        for (timestamp, name) in
            [(options.auto_now_add, "auto_now_add"), (options.auto_now, "auto_now")]
        {
            if !timestamp {
                continue;
            }
            if options.ignore {
                errors.push(Error::new(
                    ident.span(),
                    format!("bee_orm: #[bee({name})] is not valid on an ignored field"),
                ));
            }
            if options.auto {
                errors.push(Error::new(
                    ident.span(),
                    format!(
                        "bee_orm: #[bee({name})] is not valid on a database-assigned (auto) field"
                    ),
                ));
            }
        }
        if options.soft_delete && options.ignore {
            errors.push(Error::new(
                ident.span(),
                "bee_orm: #[bee(soft_delete)] is not valid on an ignored field",
            ));
        }
        if options.fk.is_some() && options.ignore {
            errors.push(Error::new(
                ident.span(),
                "bee_orm: #[bee(fk)] is not valid on an ignored field",
            ));
        }

        let column = match options.column {
            Some(literal) => {
                if !is_valid_column(&literal.value()) {
                    errors.push(Error::new(
                        literal.span(),
                        format!(
                            "bee_orm: invalid column name `{}` (expected [A-Za-z_][A-Za-z0-9_]*)",
                            literal.value()
                        ),
                    ));
                }
                literal
            }
            // `unraw`: the column of a `r#type` field is `type`, not `r#type`.
            None => LitStr::new(&ident.unraw().to_string(), ident.span()),
        };

        columns.push(Column {
            ident,
            ty: field.ty.clone(),
            column,
            pk: options.pk,
            auto: options.auto,
            ignore: options.ignore,
            auto_now_add: options.auto_now_add,
            auto_now: options.auto_now,
            soft_delete: options.soft_delete,
            sql_type: options.sql_type,
            fk: options.fk,
        });
    }
    columns
}

// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! §34 type mapping: a field's syntactic spelling to a `SqlType` variant name.
//! Anything outside the table — aliases, `u64`/`usize`, custom types — needs
//! the `#[bee(sql_type = "...")]` override, since a derive sees spellings, not
//! resolved types.

use syn::{GenericArgument, PathArguments, PathSegment, Type};

/// Splits one outer `Option<…>` off `ty`; returns whether it was there.
pub(crate) fn strip_option(ty: &Type) -> (bool, &Type) {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == "Option"
        && let PathArguments::AngleBracketed(arguments) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = arguments.args.first()
    {
        return (true, inner);
    }
    (false, ty)
}

/// The `SqlType` variant name for a spelling, matched on the last path segment
/// (§34 table) — `DateTime` also inspects its generic argument (§56).
/// `Option` must already be stripped by [`strip_option`].
pub(crate) fn spelling(ty: &Type) -> Option<&'static str> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let plain = matches!(segment.arguments, PathArguments::None);
    match segment.ident.to_string().as_str() {
        "bool" if plain => Some("Bool"),
        "i8" | "i16" | "i32" | "u8" | "u16" | "u32" if plain => Some("Int"),
        "i64" if plain => Some("BigInt"),
        "f32" if plain => Some("Real"),
        "f64" if plain => Some("Double"),
        "String" if plain => Some("Text"),
        "Vec" => vec_u8(segment).then_some("Blob"),
        // §45: only a path that actually names `serde_json` — a bare `Value`
        // is ambiguous with `bee_orm::Value` and stays unmapped.
        "Value" if plain && json_path(&path.path) => Some("Json"),
        // §56: the date/time and decimal spellings are unconditional here even
        // though the `bee_orm` feature backing them is opt-in — the `SqlType`
        // variants always exist, so the table needs no cfg. A model using them
        // without the feature fails at compile time on the missing conversion,
        // which is the documented contract.
        "NaiveDate" if plain => Some("Date"),
        "NaiveDateTime" if plain => Some("DateTime"),
        "Decimal" if plain => Some("Decimal"),
        // Only UTC maps: any other time zone needs the explicit override.
        "DateTime" if utc_datetime(segment) => Some("DateTimeTz"),
        _ => None,
    }
}

/// Whether the segment is spelled `DateTime<Utc>` — §56's only mapped time
/// zone: exactly one generic argument, its last path segment `Utc` (so
/// `chrono::Utc` counts and `Local` / `FixedOffset` / `Utc` aliases do not).
fn utc_datetime(segment: &PathSegment) -> bool {
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    arguments.args.len() == 1
        && matches!(arguments.args.first(),
            Some(GenericArgument::Type(Type::Path(inner)))
                if inner.path.segments.last().is_some_and(|segment| segment.ident == "Utc"))
}

/// Whether any segment of the path is spelled `serde_json` (covers
/// `serde_json::Value` and `serde_json::value::Value`).
fn json_path(path: &syn::Path) -> bool {
    path.segments.iter().any(|segment| segment.ident == "serde_json")
}

/// Whether the spelling's last segment is `Value` — the JSON spelling the
/// table deliberately leaves unmapped, so the error can name the exact fix.
pub(crate) fn last_segment_is_value(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Value"))
}

/// Whether the segment is spelled `Vec<u8>`.
fn vec_u8(segment: &PathSegment) -> bool {
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    matches!(arguments.args.first(),
        Some(GenericArgument::Type(Type::Path(inner)))
            if inner.path.segments.last().is_some_and(|segment| segment.ident == "u8"))
}

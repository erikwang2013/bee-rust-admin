// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, LitInt, LitStr, PathArguments, Type,
    parse_macro_input,
};

#[proc_macro_derive(Model, attributes(bee))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// 与 `bee_orm::ColumnType` 的变体一一对应。
#[derive(Clone, Copy)]
enum Ty {
    U64,
    I64,
    U32,
    I32,
    I16,
    I8,
    Bool,
    String,
    F64,
    DateTime,
    Json,
}

impl Ty {
    fn ident(self) -> proc_macro2::Ident {
        let name = match self {
            Ty::U64 => "U64",
            Ty::I64 => "I64",
            Ty::U32 => "U32",
            Ty::I32 => "I32",
            Ty::I16 => "I16",
            Ty::I8 => "I8",
            Ty::Bool => "Bool",
            Ty::String => "String",
            Ty::F64 => "F64",
            Ty::DateTime => "DateTime",
            Ty::Json => "Json",
        };
        format_ident!("{}", name)
    }
}

struct Field {
    ident: syn::Ident,
    column: String,
    ty: Ty,
    nullable: bool,
    auto: bool,
    unique: bool,
    index: bool,
    text: bool,
    len: Option<u32>,
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let mut table: Option<String> = None;
    let mut pk: Option<String> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("bee") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                table = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("pk") {
                pk = Some(meta.value()?.parse::<LitStr>()?.value());
            } else {
                return Err(meta.error("未知的 bee 容器属性（支持 table / pk）"));
            }
            Ok(())
        })?;
    }
    let table = table.unwrap_or_else(|| format!("{}s", to_snake(&name.to_string())));

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => return Err(syn::Error::new_spanned(name, "Model 只支持具名字段结构体")),
        },
        _ => return Err(syn::Error::new_spanned(name, "Model 只支持结构体")),
    };

    let mut infos = Vec::new();
    for f in fields {
        let ident = f.ident.clone().expect("具名字段");
        let (ty, nullable) = type_to_col(&f.ty)?;
        let mut info = Field {
            column: ident.to_string(),
            ident,
            ty,
            nullable,
            auto: false,
            unique: false,
            index: false,
            text: false,
            len: None,
        };
        for attr in &f.attrs {
            if !attr.path().is_ident("bee") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("auto") {
                    info.auto = true;
                } else if meta.path.is_ident("unique") {
                    info.unique = true;
                } else if meta.path.is_ident("index") {
                    info.index = true;
                } else if meta.path.is_ident("text") {
                    info.text = true;
                } else if meta.path.is_ident("len") {
                    info.len = Some(meta.value()?.parse::<LitInt>()?.base10_parse()?);
                } else {
                    return Err(meta.error(
                        "未知的 bee 字段属性（支持 auto/unique/index/text/len）",
                    ));
                }
                Ok(())
            })?;
        }
        infos.push(info);
    }

    let pk = pk.or_else(|| infos.iter().find(|f| f.column == "id").map(|f| f.column.clone()));
    let table_str = table.as_str();
    let pk_tokens = match &pk {
        Some(p) => quote!(Some(#p)),
        None => quote!(None),
    };

    let col_metas = infos.iter().map(|f| {
        let (n, t) = (&f.column, f.ty.ident());
        let (auto, unique, index, text, nullable) =
            (f.auto, f.unique, f.index, f.text, f.nullable);
        let len = match f.len {
            Some(n) => quote!(Some(#n)),
            None => quote!(None),
        };
        quote! {
            bee_orm::ColumnMeta {
                name: #n, ty: bee_orm::ColumnType::#t, auto: #auto, unique: #unique,
                index: #index, nullable: #nullable, text: #text, len: #len,
            }
        }
    });

    let from_row_fields = infos.iter().map(|f| {
        let (ident, col) = (&f.ident, &f.column);
        quote! { #ident: bee_orm::__private::Row::try_get(row, #col)? }
    });

    let insert_binds = infos.iter().filter(|f| !f.auto).map(|f| {
        let ident = &f.ident;
        quote! { sep.push_bind(self.#ident.clone()); }
    });

    let update_sets: Vec<_> = infos.iter().filter(|f| !f.auto).collect();
    // 每列一个完整片段（首列不带分隔符）——不要用两个长度不等的迭代器做 quote 重复，
    // quote 的重复要求所有迭代器等长，否则宏展开处直接报错。
    let update_chunks: Vec<_> = update_sets
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let (ident, col) = (&f.ident, &f.column);
            if i == 0 {
                quote! { qb.push(#col).push(" = ").push_bind(self.#ident.clone()); }
            } else {
                quote! { qb.push(", ").push(#col).push(" = ").push_bind(self.#ident.clone()); }
            }
        })
        .collect();

    // 主键列名用于 UPDATE 尾部；没有主键时不生成（Db::update 会先报错）。
    let update_where = match pk.as_deref().and_then(|p| infos.iter().find(|f| f.column == p)) {
        Some(f) => {
            let (ident, col) = (&f.ident, &f.column);
            quote! { qb.push(" WHERE ").push(#col).push(" = ").push_bind(self.#ident.clone()); }
        }
        None => quote! {},
    };

    let auto_setter = if infos.iter().any(|f| f.auto) {
        let f = infos.iter().find(|f| f.auto).expect("auto 字段");
        let ident = &f.ident;
        quote! { fn set_auto_pk(&mut self, id: u64) { self.#ident = id; } }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl bee_orm::Model for #name {
            const META: bee_orm::ModelMeta = bee_orm::ModelMeta {
                table: #table_str,
                pk: #pk_tokens,
                columns: &[ #(#col_metas),* ],
            };

            fn from_row(row: &bee_orm::__private::MySqlRow) -> Result<Self, bee_orm::__private::sqlx::Error> {
                Ok(Self { #(#from_row_fields),* })
            }

            fn bind_insert<'q>(
                &self,
                mut qb: bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql>,
            ) -> bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql> {
                {
                    let mut sep = qb.separated(", ");
                    #(#insert_binds)*
                }
                qb
            }

            fn bind_update<'q>(
                &self,
                mut qb: bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql>,
            ) -> bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql> {
                qb.push("SET ");
                #(#update_chunks)*
                #update_where
                qb
            }

            #auto_setter
        }

        impl #name {
            pub fn query() -> bee_orm::QuerySet<Self> {
                bee_orm::QuerySet::new(#table_str)
            }

            pub fn table_name() -> &'static str {
                #table_str
            }
        }
    })
}

/// `Option<T>` 展开为内层类型 + 可空；未知类型报编译错误。
fn type_to_col(ty: &Type) -> syn::Result<(Ty, bool)> {
    let Type::Path(p) = ty else {
        return Err(syn::Error::new_spanned(ty, "不支持的字段类型"));
    };
    let seg = p.path.segments.last().expect("类型路径非空");
    let name = seg.ident.to_string();
    if name == "Option" {
        let PathArguments::AngleBracketed(args) = &seg.arguments else {
            return Err(syn::Error::new_spanned(ty, "Option 需要类型参数"));
        };
        let GenericArgument::Type(inner) = args.args.first().expect("Option 有类型参数") else {
            return Err(syn::Error::new_spanned(ty, "Option 需要类型参数"));
        };
        let (t, _) = type_to_col(inner)?;
        return Ok((t, true));
    }
    let t = match name.as_str() {
        "u64" => Ty::U64,
        "i64" => Ty::I64,
        "u32" => Ty::U32,
        "i32" => Ty::I32,
        "i16" => Ty::I16,
        "i8" => Ty::I8,
        "bool" => Ty::Bool,
        "String" => Ty::String,
        "f64" => Ty::F64,
        "NaiveDateTime" => Ty::DateTime,
        "Value" => Ty::Json,
        other => {
            return Err(syn::Error::new_spanned(
                ty,
                format!("不支持的字段类型 `{other}`（支持 u64/i64/u32/i32/i16/i8/bool/String/f64/NaiveDateTime/serde_json::Value 及 Option<T>）"),
            ));
        }
    };
    Ok((t, false))
}

/// `UtAdmin` → `ut_admin`。默认表名 = snake_case + "s"（`User` → `users`，与旧行为兼容；
/// 复数化是朴素加 s，不处理 `Address` → `addresss` 这类，需要就用 `#[bee(table = "…")]`）。
fn to_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

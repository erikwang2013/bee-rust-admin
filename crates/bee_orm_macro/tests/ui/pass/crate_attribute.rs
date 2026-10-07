// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Spec §61.2: `#[bee(crate = "…")]` — the expansion prefixes every emitted
//! path with the attribute's path, so a model reached through a re-export
//! compiles. The inner module shadows the bare `bee_orm` name on purpose: were
//! the attribute ignored, the emitted `bee_orm::…` paths would resolve to the
//! empty local module below and this case would stop compiling.
#![allow(dead_code)]

use bee_orm::Model;

#[derive(Model)]
struct Plain {
    id: i64,
}

mod reexported {
    // Shadows the extern-prelude `bee_orm` here: the real crate is reachable
    // only through `orm`, and only the attribute makes the expansion use it.
    mod bee_orm {}

    use ::bee_orm as orm;
    use orm::Model;

    #[derive(orm::Model)]
    #[bee(crate = "orm")]
    pub struct Tag {
        pub id: i64,
        pub user_id: i64,
    }

    pub fn exercise() -> &'static str {
        fn assert_model<T: orm::Model>() {}
        assert_model::<Tag>();
        let _ = Tag::query();
        assert_eq!(Tag::columns().len(), 2);
        assert_eq!(Tag::pk_column(), "id");
        Tag::table_name()
    }
}

fn main() {
    // The attribute-less default still expands to the real crate.
    assert_eq!(Plain::table_name(), "plains");
    assert_eq!(Plain::columns().len(), 1);
    assert_eq!(Plain { id: 7 }.insert_values().len(), 1);

    assert_eq!(reexported::exercise(), "tags");
}

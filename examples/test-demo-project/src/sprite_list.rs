//! Shared between `src/` and `build.rs` via `#[path]` include, since build
//! scripts are their own crate root and can't `use crate::...`.
use std::collections::BTreeMap;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SpriteEntry {
    pub path: String,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct SpriteList {
    pub sprites: BTreeMap<String, SpriteEntry>,
}

impl SpriteList {
    pub fn insert(&mut self, sprite_name: String, sprite_entry: SpriteEntry) {
        self.sprites.insert(sprite_name, sprite_entry);
    }
}

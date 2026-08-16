use std::error::Error;
use std::path::{Path, PathBuf};

// we share the same data structures with the main project to make it easier for us.
#[path = "src/sprite_list.rs"]
mod sprite_list;
use sprite_list::{SpriteEntry, SpriteList};

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp"];

const ASSETS_DIR: &str = "assets";
const SPRITE_LIST_PATH: &str = "assets/sprites.json";

fn main() -> Result<(), Box<dyn Error>> {
    // Cargo scans directories recursively, so this covers new files appearing.
    println!("cargo::rerun-if-changed={ASSETS_DIR}");

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets_dir = manifest_dir.join(ASSETS_DIR);
    let sprite_list_path = manifest_dir.join(SPRITE_LIST_PATH);

    let mut sprite_list = SpriteList::default();
    collect_images(&assets_dir, &assets_dir, &mut sprite_list);

    let manifest = serde_json::to_string_pretty(&sprite_list)?;
    if std::fs::read_to_string(&sprite_list_path).ok().as_deref() != Some(manifest.as_str()) {
        std::fs::write(&sprite_list_path, &manifest)?;
    }
    Ok(())
}

/// Walks `dir`, pushing every image file as a `/`-separated path relative to `root`.
fn collect_images(root: &Path, dir: &Path, out: &mut SpriteList) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(root, &path, out);
            continue;
        }

        let is_image = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()));

        if is_image
            && let Ok(relative) = path.strip_prefix(root)
            && let Some(relative) = relative.to_str()
        {
            // The wasm build fetches these over HTTP, so separators must be `/`
            // even though this script runs on Windows.
            let relative = relative.replace('\\', "/");
            let key = unique_key(&relative, out);
            out.insert(key, SpriteEntry { path: relative });
        }
    }
}

/// Default key for a sprite: its file stem, suffixed if that name is already taken.
fn unique_key(relative_path: &str, sprite_list: &SpriteList) -> String {
    let stem = Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(relative_path);

    let mut key = stem.to_string();
    let mut suffix = 2;
    while sprite_list.sprites.contains_key(&key) {
        key = format!("{stem}_{suffix}");
        suffix += 1;
    }

    key
}
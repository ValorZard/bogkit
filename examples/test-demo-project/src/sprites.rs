use crate::asset_handler::fetch_asset_bytes;
use crate::sprite_list::SpriteList;
use kiss3d::prelude::*;

pub async fn preload_sprites(manifest_path: &str, texture_manager: &mut TextureManager) {
    let bytes = fetch_asset_bytes(manifest_path)
        .await
        .expect("should be able to fetch the sprite manifest");
    let manifest: SpriteList =
        serde_json::from_slice(&bytes).expect("sprite manifest should be valid JSON");

    for (name, entry) in &manifest.sprites {
        let image_bytes = fetch_asset_bytes(&entry.path).await.unwrap_or_else(|e| {
            panic!("failed to fetch sprite '{name}' from '{}': {e}", entry.path)
        });

        texture_manager.add_image_from_memory_pixelated(&image_bytes, name);
    }
}
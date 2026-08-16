use std::error::Error;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AssetFetchError {
    #[error("failed to request asset '{path}'")]
    #[cfg(target_family = "wasm")]
    PathFailure { path: String },
    #[error("asset '{path}' returned HTTP {status}")]
    #[cfg(target_family = "wasm")]
    HTTPError { path: String, status: u16 },
    #[error("failed to read asset '{path}' body")]
    BinaryReadFailure { path: String, error: Box<dyn Error> },
    #[error("asset '{path}' not found; searched: {searched}")]
    #[cfg(not(target_family = "wasm"))]
    NotFound { path: String, searched: String },
    #[error("the environment this exe is doesn't seem to work")]
    #[cfg(not(target_family = "wasm"))]
    InvalidEnvironment,
}

#[cfg(target_arch = "wasm32")]
async fn fetch_asset_bytes_wasm(relative_path: &str) -> Result<Vec<u8>, AssetFetchError> {
    use gloo_net::http::Request;

    let path = format!("assets/{relative_path}");
    let Ok(response) = Request::get(&path).send().await else {
        return Err(AssetFetchError::PathFailure { path });
    };

    if !response.ok() {
        return Err(AssetFetchError::HTTPError {
            path,
            status: response.status(),
        });
    }

    match response.binary().await {
        Ok(vec) => {
            return Ok(vec);
        }
        Err(error) => {
            return Err(AssetFetchError::BinaryReadFailure {
                path,
                error: error.into(),
            });
        }
    }
}

/// Check through all of the possible locations the assets folder might be
#[cfg(not(target_arch = "wasm32"))]
fn possible_assets_directories() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut paths: Vec<PathBuf> = Vec::new();

    // Shipped builds: `upload_game.sh` copies `assets/` next to the binary.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        paths.push(dir.to_path_buf());
    }

    // This is the path to the Cargo.toml that this game is launched from.
    // the assets/ folder should be right next to Cargo.toml in the source directory.
    // (This should only really happen during development)
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    // we could also check the directory that this exe was launched from, but that seems unnecessary.

    paths
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch_asset_bytes_native(relative_path: &str) -> Result<Vec<u8>, AssetFetchError> {
    let paths = possible_assets_directories();
    if paths.is_empty() {
        return Err(AssetFetchError::InvalidEnvironment);
    }

    for path in &paths {
        let candidate = path.join("assets").join(relative_path);
        match std::fs::read(&candidate) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AssetFetchError::BinaryReadFailure {
                    path: candidate.display().to_string(),
                    error: error.into(),
                });
            }
        }
    }

    let searched = paths
        .iter()
        .map(|root| root.join("assets").display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    Err(AssetFetchError::NotFound {
        path: relative_path.to_string(),
        searched,
    })
}

/// Fetches an asset's raw bytes given a path relative to the `assets/` folder,
/// working the same way on both the wasm32 (browser) and native builds.
///
/// - On wasm32, this issues an HTTP GET relative to the page URL,
///   so it relies on `Trunk.toml`'s `public_url = "."`
///   and the assets actually being copied into `dist/`
///   (see the `copy-dir` link in `index.html`)
///   this is what lets it keep working once itch.io serves the game from a hashed subpath.
/// - On native, this reads from disk next to the executable,
///   since `upload_client.sh` ships an `assets/` folder alongside the binary.
///   It falls back to `CARGO_MANIFEST_DIR` so `cargo run` works without staging
///   assets next to `target/debug/client.exe`.
pub async fn fetch_asset_bytes(relative_path: &str) -> Result<Vec<u8>, AssetFetchError> {
    cfg_select! {
        target_arch = "wasm32" => fetch_asset_bytes_wasm(relative_path).await,
        _ => fetch_asset_bytes_native(relative_path),
    }
}

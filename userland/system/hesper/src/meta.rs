use vconfig::ConfigError;
use vconfig::manifest::{
    AppManifest,
    EntrypointMetadata,
    parse_manifest,
    select_entrypoint as config_select_entrypoint,
};
use vstd::prelude::*;

fn config_error(error: ConfigError) -> Error {
    match error {
        ConfigError::Invalid(message) => Error::invalid_argument(message),
        ConfigError::Parse(message) => Error::invalid_encoding(message),
        ConfigError::NotFound(message) => Error::not_found(message),
    }
}

pub fn load_manifest(bundle: &str) -> Result<AppManifest, Error> {
    let manifest_path = format!("{}/manifest.toml", bundle);
    let manifest_file = File::open(&Path::new(manifest_path.as_str()))?;
    let manifest_str = manifest_file.read_to_string()?;

    parse_manifest(manifest_str.as_str(), manifest_path.as_str()).map_err(config_error)
}

pub fn select_entrypoint(manifest: &AppManifest, name: &str) -> Result<EntrypointMetadata, Error> {
    config_select_entrypoint(manifest, name).map_err(config_error)
}

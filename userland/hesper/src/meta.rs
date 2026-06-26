use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use vespertine_std::fs::{
    File,
    Path,
};
use vespertine_std::{
    Error,
    Read,
};

use alloc::collections::BTreeMap;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppManifest {
    pub application: AppMetadata,
    pub entrypoints: BTreeMap<String, EntrypointMetadata>,

    #[serde(default)]
    pub permissions: PermissionMetadata,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppMetadata {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EntrypointMetadata {
    pub binary: String,
    pub input: String,
    pub modes: Vec<String>,

    #[serde(default = "default_io_mode")]
    pub default: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedApp {
    pub app_id: String,
    pub bundle: String,
    pub entrypoint_name: String,
    pub manifest: AppManifest,
    pub entrypoint: EntrypointMetadata,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PermissionMetadata {
    #[serde(default = "default_filesystem_access")]
    pub filesystem: String,

    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl Default for PermissionMetadata {
    fn default() -> Self { Self { filesystem: default_filesystem_access(), capabilities: Vec::new() } }
}

fn default_filesystem_access() -> String { String::from("read-only") }

fn default_io_mode() -> String { String::from("text") }

fn validate_component(value: &str, desc: &str) -> Result<(), Error> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') || value.as_bytes().contains(&0) {
        return Err(Error::invalid_argument(format!("invalid {}", desc).into()));
    }
    if value.len() > 254 {
        return Err(Error::name_too_long(format!("{} is too long", desc).into()));
    }
    Ok(())
}

pub fn load_manifest(bundle: &str) -> Result<AppManifest, Error> {
    let manifest_path = format!("{}/manifest.toml", bundle);
    let manifest_file = File::open(&Path::new(manifest_path.as_str()))?;
    let manifest_str = manifest_file.read_to_string()?;

    let manifest = toml::from_str::<AppManifest>(manifest_str.as_str())
        .map_err(|e| Error::invalid_encoding(format!("could not parse file into toml: {:?}", e).into()))?;

    if manifest.application.id.is_empty() {
        return Err(Error::invalid_argument("application ID cannot be empty".into()));
    }

    if manifest.entrypoints.is_empty() {
        return Err(Error::invalid_argument("application must declare at least one entrypoint".into()));
    }

    for (name, entrypoint) in &manifest.entrypoints {
        validate_component(name, "entrypoint name")?;
        validate_component(&entrypoint.binary, "application binary name")?;
    }

    Ok(manifest)
}

pub fn select_entrypoint(manifest: &AppManifest, name: &str) -> Result<EntrypointMetadata, Error> {
    manifest.entrypoints.get(name).cloned()
        .ok_or_else(|| Error::not_found("application entrypoint was not found".into()))
}

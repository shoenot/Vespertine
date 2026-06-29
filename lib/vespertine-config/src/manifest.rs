use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde::Deserialize;

use crate::ConfigError;

#[derive(Debug, Clone, Deserialize)]
pub struct AppManifest {
    pub application: AppMetadata,
    pub entrypoints: BTreeMap<String, EntrypointMetadata>,

    #[serde(default)]
    pub permissions: PermissionMetadata,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppMetadata {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntrypointMetadata {
    pub binary: String,
    pub input: String,
    pub modes: Vec<String>,

    #[serde(default = "default_io_mode")]
    pub default: String,
}

#[derive(Debug, Clone, Deserialize)]
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

fn validate_component(value: &str, desc: &str) -> Result<(), ConfigError> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') || value.as_bytes().contains(&0) {
        return Err(ConfigError::invalid(format!("invalid {}", desc)));
    }

    if value.len() > 254 {
        return Err(ConfigError::invalid(format!("{} is too long", desc)));
    }

    Ok(())
}

fn validate_app_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty() || id.contains('/') || id.as_bytes().contains(&0) {
        return Err(ConfigError::invalid("invalid app ID".into()));
    }

    Ok(())
}

pub fn parse_manifest(text: &str, description: &str) -> Result<AppManifest, ConfigError> {
    let manifest = toml::from_str::<AppManifest>(text)
        .map_err(|error| ConfigError::parse(format!("invalid application manifest {}: {:?}", description, error)))?;

    validate_manifest(&manifest)?;

    Ok(manifest)
}

pub fn validate_manifest(manifest: &AppManifest) -> Result<(), ConfigError> {
    validate_app_id(&manifest.application.id)?;

    if manifest.application.name.is_empty() {
        return Err(ConfigError::invalid("application display name cannot be empty".into()));
    }

    if manifest.entrypoints.is_empty() {
        return Err(ConfigError::invalid("application must declare at least one entrypoint".into()));
    }

    for (name, entrypoint) in &manifest.entrypoints {
        validate_component(name, "entrypoint name")?;
        validate_component(&entrypoint.binary, "application binary name")?;
    }

    Ok(())
}

pub fn select_entrypoint(manifest: &AppManifest, name: &str) -> Result<EntrypointMetadata, ConfigError> {
    manifest.entrypoints.get(name).cloned().ok_or_else(|| ConfigError::not_found("application entrypoint was not found".into()))
}

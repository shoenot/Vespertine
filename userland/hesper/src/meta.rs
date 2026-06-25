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

#[derive(Debug, serde::Deserialize)]
pub struct AppManifest {
    pub application: AppMetadata,
    pub io: IoMetadata,

    #[serde(default)]
    pub permissions: PermissionMetadata,
}

#[derive(Debug, serde::Deserialize)]
pub struct AppMetadata {
    pub name: String,
    pub binary: String,
    pub id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct IoMetadata {
    pub input: String,
    pub output: String,
}

#[derive(Debug, serde::Deserialize)]
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

fn validate_component(value: &str, desc: &str) -> Result<(), Error> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') || value.as_bytes().contains(&0) {
        return Err(Error::invalid_argument(format!("invalid {}", desc).into()));
    }
    if value.len() > 254 {
        return Err(Error::name_too_long(format!("{} is too long", desc).into()));
    }
    Ok(())
}

pub fn get_metadata(name: &str) -> Result<AppManifest, Error> {
    let app_path = format!("/Programs/{}.app/manifest.toml", name);
    let manifest_file = File::open(&Path::new(app_path.as_str()))?;
    let manifest_str = manifest_file.read_to_string()?;

    let manifest = toml::from_str::<AppManifest>(manifest_str.as_str())
        .map_err(|e| Error::invalid_encoding(format!("could not parse file into toml: {:?}", e).into()))?;

    validate_component(&manifest.application.binary, "application binary name")?;
    if manifest.application.id.is_empty() {
        return Err(Error::invalid_argument("application ID cannot be empty".into()));
    }
    Ok(manifest)
}

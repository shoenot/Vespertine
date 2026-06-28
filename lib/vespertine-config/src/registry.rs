use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{
    String,
    ToString,
};
use alloc::vec::Vec;

use serde::Deserialize;
use toml::value::Datetime;

use crate::ConfigError;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RegistryIndex {
    pub version: u32,
    pub applications: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RegistryFileToml {
    pub version: u32,
    pub application: Vec<ApplicationRecord>,
    pub installation: InstallationMetadataToml,
}

#[derive(Debug, Clone)]
pub struct RegistryFile {
    pub version: u32,
    pub application: Vec<ApplicationRecord>,
    pub installation: InstallationMetadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ApplicationRecord {
    pub id: String,
    pub bundle: String,
    pub default_entrypoint: String,

    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct InstallationMetadataToml {
    pub installed_ts: Datetime,
    pub updated_ts: Datetime,
}

#[derive(Debug, Clone)]
pub struct InstallationMetadata {
    pub installed_ts: String,
    pub updated_ts: String,
}

pub fn parse_registry_index(text: &str) -> Result<RegistryIndex, ConfigError> {
    toml::from_str::<RegistryIndex>(text)
        .map_err(|error| ConfigError::parse(format!("invalid app registry index: {:?}", error)))
}

pub fn parse_registry_file(text: &str, path: &str) -> Result<RegistryFile, ConfigError> {
    let parsed = toml::from_str::<RegistryFileToml>(text)
        .map_err(|error| ConfigError::parse(format!("invalid app registry file {}: {:?}", path, error)))?;

    Ok(RegistryFile {
        version: parsed.version,
        application: parsed.application,
        installation: InstallationMetadata {
            installed_ts: parsed.installation.installed_ts.to_string(),
            updated_ts: parsed.installation.updated_ts.to_string(),
        },
    })
}

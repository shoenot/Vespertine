use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde::Deserialize;

use crate::ConfigError;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct GrantSet {
    #[serde(default)]
    pub root_rights: Vec<String>,

    #[serde(default)]
    pub cwd_rights: Vec<String>,

    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl GrantSet {
    pub fn extend(&mut self, other: &GrantSet) {
        self.root_rights.extend(other.root_rights.iter().cloned());
        self.cwd_rights.extend(other.cwd_rights.iter().cloned());
        self.capabilities.extend(other.capabilities.iter().cloned());
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchetypeFile {
    pub version: u32,
    pub defaults: GrantSet,

    #[serde(default)]
    pub archetype: BTreeMap<String, GrantSet>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationGrant {
    pub id: String,
    pub bundle: String,

    #[serde(default)]
    pub archetype: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantFile {
    pub application: ApplicationGrant,

    #[serde(default)]
    pub grants: GrantSet,
}

pub fn parse_archetype_file(text: &str) -> Result<ArchetypeFile, ConfigError> {
    toml::from_str::<ArchetypeFile>(text)
        .map_err(|error| ConfigError::parse(format!("invalid launcher archetype policy: {:?}", error)))
}

pub fn parse_grant_file(text: &str, path: &str) -> Result<GrantFile, ConfigError> {
    toml::from_str::<GrantFile>(text)
        .map_err(|error| ConfigError::parse(format!("invalid grant file {}: {:?}", path, error)))
}

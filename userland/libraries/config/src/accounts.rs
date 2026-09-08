use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde::Deserialize;

use crate::ConfigError;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AccountIndex {
    pub version: u32,
    pub default: String,
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AccountFile {
    pub version: u32,
    pub user: UserRecord,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct UserRecord {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub home: String,

    #[serde(default)]
    pub roles: Vec<String>,
}

pub fn parse_account_index(text: &str) -> Result<AccountIndex, ConfigError> {
    toml::from_str::<AccountIndex>(text).map_err(|error| ConfigError::parse(format!("invalid account index: {:?}", error)))
}

pub fn parse_account_file(text: &str, path: &str) -> Result<AccountFile, ConfigError> {
    toml::from_str::<AccountFile>(text).map_err(|error| ConfigError::parse(format!("invalid account file {}: {:?}", path, error)))
}

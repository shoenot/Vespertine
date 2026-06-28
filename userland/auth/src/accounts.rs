extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{
    String,
    ToString,
};
use alloc::vec::Vec;

use config::ConfigError;
use config::accounts::{
    UserRecord,
    parse_account_file,
    parse_account_index,
};
use vespertine_abi::{
    SYSTEM_USER,
    UserID,
};
use vespertine_std::auth::AccountInfo;
use vespertine_std::fs::{
    File,
    Path,
};
use vespertine_std::typed::named_user_value;
use vespertine_std::{
    Error,
    Read,
};

const ACCOUNT_INDEX: &str = "/System/Accounts/index.toml";
const ACCOUNT_USERS: &str = "/System/Accounts/Users";

#[derive(Debug, Clone)]
pub struct UserAccount {
    pub id: UserID,
    pub name: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub home: String,
    pub roles: Vec<String>,
}

pub struct AccountStore {
    default_user: String,
    users_by_name: BTreeMap<String, UserAccount>,
    users_by_id: BTreeMap<u32, String>,
}

fn config_error(error: ConfigError) -> Error {
    match error {
        ConfigError::Invalid(message) => Error::invalid_argument(message),
        ConfigError::Parse(message) => Error::invalid_encoding(message),
        ConfigError::NotFound(message) => Error::not_found(message),
    }
}

fn read_text(path: &str) -> Result<String, Error> {
    File::open(&Path::new(path))?.read_to_string()
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.as_bytes().contains(&0) {
        return Err(Error::invalid_argument("invalid account name".into()));
    }

    if name.len() > 64 {
        return Err(Error::name_too_long("account name is too long".into()));
    }

    Ok(())
}

fn validate_record(record: &UserRecord) -> Result<(), Error> {
    validate_name(&record.name)?;

    if record.id == SYSTEM_USER.0 && record.name != "system" {
        return Err(Error::invalid_argument("system user must be named system".into()));
    }

    if record.name == "system" && record.id != SYSTEM_USER.0 {
        return Err(Error::invalid_argument("system account must use user ID 0".into()));
    }

    if record.home.is_empty() || record.home.contains('\0') {
        return Err(Error::invalid_argument("invalid user home directory".into()));
    }

    if record.id == SYSTEM_USER.0 {
        if record.home != "/System" {
            return Err(Error::invalid_argument("system account home must be /System".into()));
        }
    } else if !record.home.starts_with("/Users/") {
        return Err(Error::invalid_argument("user home directory must be under /Users".into()));
    }


    if record.display_name.is_empty() {
        return Err(Error::invalid_argument("user display name cannot be empty".into()));
    }

    for role in &record.roles {
        validate_name(role)?;
    }

    Ok(())
}

impl UserAccount {
    pub fn info(&self) -> Result<AccountInfo, Error> {
        Ok(AccountInfo {
            user: named_user_value(self.id.0, &self.name, &self.display_name, &self.first_name, &self.last_name)?,
            home: self.home.clone(),
            roles: self.roles.clone(),
        })
    }
}

impl AccountStore {
    pub fn load() -> Result<Self, Error> {
        let index_text = read_text(ACCOUNT_INDEX)?;
        let index = parse_account_index(&index_text).map_err(config_error)?;

        if index.version != 1 {
            return Err(Error::invalid_argument("unsupported account index version".into()));
        }

        validate_name(&index.default)?;

        let mut users_by_name = BTreeMap::new();
        let mut users_by_id = BTreeMap::new();

        for name in index.users {
            validate_name(&name)?;

            if users_by_name.contains_key(&name) {
                return Err(Error::invalid_argument(format!("duplicate account {}", name).into()));
            }

            let path = format!("{}/{}.toml", ACCOUNT_USERS, name);
            let text = read_text(&path)?;
            let file = parse_account_file(&text, &path).map_err(config_error)?;

            if file.version != 1 {
                return Err(Error::invalid_argument(format!("unsupported account file version in {}", path).into()));
            }

            validate_record(&file.user)?;

            if file.user.name != name {
                return Err(Error::invalid_argument(format!("account file {} declared mismatched user {}", path, file.user.name).into()));
            }

            if users_by_id.contains_key(&file.user.id) {
                return Err(Error::invalid_argument(format!("duplicate user ID {}", file.user.id).into()));
            }

            let account = UserAccount {
                id: UserID(file.user.id),
                name: file.user.name,
                display_name: file.user.display_name,
                first_name: file.user.first_name,
                last_name: file.user.last_name,
                home: file.user.home,
                roles: file.user.roles,
            };

            users_by_id.insert(account.id.0, account.name.clone());
            users_by_name.insert(account.name.clone(), account);
        }

        if !users_by_name.contains_key(&index.default) {
            return Err(Error::invalid_argument("default user does not exist".into()));
        }

        Ok(Self {
            default_user: index.default,
            users_by_name,
            users_by_id,
        })
    }

    pub fn default_user(&self) -> Result<&UserAccount, Error> {
        self.users_by_name.get(&self.default_user)
            .ok_or_else(|| Error::not_found("default user does not exist".into()))
    }

    pub fn by_id(&self, user: UserID) -> Result<&UserAccount, Error> {
        let name = self.users_by_id.get(&user.0)
            .ok_or_else(|| Error::not_found("user account does not exist".into()))?;

        self.users_by_name.get(name)
            .ok_or_else(|| Error::not_found("user account does not exist".into()))
    }

    pub fn by_name(&self, name: &str) -> Result<&UserAccount, Error> {
        self.users_by_name.get(name)
            .ok_or_else(|| Error::not_found("user account does not exist".into()))
    }
}


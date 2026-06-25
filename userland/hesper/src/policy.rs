use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{
    String,
    ToString,
};
use alloc::vec::Vec;

use serde::Deserialize;
use vespertine_abi::tag::CAP_APP_TERMCTRL;
use vespertine_abi::{
    AccessRights,
    CapabilityID,
};
use vespertine_std::fs::{
    Dir,
    EntryKind,
    File,
    Path,
};
use vespertine_std::{
    Error,
    Read,
};

use crate::meta::AppManifest;

const ARCHETYPES_PATH: &str = "/System/Policy/archetypes.toml";

const GRANTS_PATH: &str = "/System/Policy/Grants";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct GrantSet {
    #[serde(default)]
    root_rights: Vec<String>,

    #[serde(default)]
    cwd_rights: Vec<String>,

    #[serde(default)]
    capabilities: Vec<String>,
}

impl GrantSet {
    fn extend(&mut self, other: &GrantSet) {
        self.root_rights.extend(other.root_rights.iter().cloned());
        self.cwd_rights.extend(other.cwd_rights.iter().cloned());
        self.capabilities.extend(other.capabilities.iter().cloned());
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchetypeFile {
    version: u32,
    defaults: GrantSet,

    #[serde(default)]
    archetype: BTreeMap<String, GrantSet>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationGrant {
    id: String,
    bundle: String,

    #[serde(default)]
    archetype: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantFile {
    application: ApplicationGrant,

    #[serde(default)]
    grants: GrantSet,
}

#[derive(Debug)]
pub struct LaunchPolicy {
    pub root_rights: AccessRights,
    pub cwd_rights: AccessRights,
    pub capabilities: Vec<CapabilityPolicy>,
}

pub struct PolicyStore {
    defaults: GrantSet,
    archetypes: BTreeMap<String, GrantSet>,
    applications: BTreeMap<String, GrantFile>,
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityPolicy {
    pub capability: CapabilityID,
    pub rights: AccessRights,
}

fn read_text(path: &str) -> Result<String, Error> { File::open(&Path::new(path))?.read_to_string() }

impl PolicyStore {
    pub fn load() -> Result<Self, Error> {
        let archetypes_text = read_text(ARCHETYPES_PATH)?;

        let archetypes = toml::from_str::<ArchetypeFile>(&archetypes_text)
            .map_err(|error| Error::invalid_encoding(format!("invalid launcher archetype policy: {:?}", error,).into()))?;

        if archetypes.version != 1 {
            return Err(Error::invalid_argument("unsupported launcher policy version".into()));
        }

        let grants_dir = Dir::open(&Path::new(GRANTS_PATH))?;
        let mut applications = BTreeMap::new();

        for entry in grants_dir.list()? {
            if !matches!(entry.kind, EntryKind::File) {
                continue;
            }

            let Some(bundle_from_filename) = entry.name.strip_suffix(".toml") else {
                continue;
            };

            let path = format!("{}/{}", GRANTS_PATH, entry.name,);

            let text = read_text(&path)?;

            let grant = toml::from_str::<GrantFile>(&text)
                .map_err(|error| Error::invalid_encoding(format!("invalid grant file {}: {:?}", path, error,).into()))?;

            if grant.application.bundle != bundle_from_filename {
                return Err(Error::invalid_argument(format!("grant filename does not match bundle {}", grant.application.bundle,).into()));
            }

            if let Some(archetype) = grant.application.archetype.as_ref() {
                if !archetypes.archetype.contains_key(archetype) {
                    return Err(Error::invalid_argument(format!("unknown archetype {}", archetype,).into()));
                }
            }

            let bundle = grant.application.bundle.clone();

            if applications.insert(bundle.clone(), grant).is_some() {
                return Err(Error::invalid_argument(format!("duplicate policy for bundle {}", bundle,).into()));
            }
        }

        let store = Self { defaults: archetypes.defaults, archetypes: archetypes.archetype, applications };

        store.validate()?;
        Ok(store)
    }
}

fn parse_rights(names: &[String]) -> Result<AccessRights, Error> {
    let mut rights = AccessRights::new();

    for name in names {
        let right = match name.as_str() {
            "read" => AccessRights::READ,
            "write" => AccessRights::WRITE,
            "execute" => AccessRights::EXECUTE,
            "create" => AccessRights::CREATE,
            "remove" => AccessRights::REMOVE,
            "traverse" => AccessRights::TRAVERSE,
            "list" => AccessRights::LIST,
            "mutate" => AccessRights::MUTATE,

            _ => {
                return Err(Error::invalid_argument(format!("unknown access right {}", name).into()));
            }
        };

        rights = rights | right;
    }

    Ok(rights)
}

fn resolve_capability(name: &str) -> Result<CapabilityPolicy, Error> {
    match name {
        "term-control" => Ok(CapabilityPolicy { capability: CAP_APP_TERMCTRL, rights: AccessRights::READ | AccessRights::WRITE }),

        _ => Err(Error::invalid_argument(format!("unknown capability {}", name,).into())),
    }
}

fn validate_capability(name: &str) -> Result<(), Error> { resolve_capability(name).map(|_| ()) }

impl PolicyStore {
    fn validate(&self) -> Result<(), Error> {
        parse_rights(&self.defaults.root_rights)?;
        parse_rights(&self.defaults.cwd_rights)?;

        for capability in &self.defaults.capabilities {
            validate_capability(capability)?;
        }

        for archetype in self.archetypes.values() {
            parse_rights(&archetype.root_rights)?;
            parse_rights(&archetype.cwd_rights)?;

            for capability in &archetype.capabilities {
                validate_capability(capability)?;
            }
        }

        for grant in self.applications.values() {
            parse_rights(&grant.grants.root_rights)?;
            parse_rights(&grant.grants.cwd_rights)?;

            for capability in &grant.grants.capabilities {
                validate_capability(capability)?;
            }
        }

        Ok(())
    }
}

fn requested_root_rights(manifest: &AppManifest) -> Result<AccessRights, Error> {
    match manifest.permissions.filesystem.as_str() {
        "read-only" => Ok(AccessRights::READ | AccessRights::TRAVERSE | AccessRights::LIST),

        "mutable" => Ok(AccessRights::READ |
            AccessRights::WRITE |
            AccessRights::CREATE |
            AccessRights::REMOVE |
            AccessRights::TRAVERSE |
            AccessRights::LIST),

        _ => Err(Error::invalid_argument("unknown filesystem permission request".into())),
    }
}

impl PolicyStore {
    pub fn resolve(&self, bundle: &str, manifest: &AppManifest) -> Result<LaunchPolicy, Error> {
        let mut maximum = self.defaults.clone();

        if let Some(application) = self.applications.get(bundle) {
            if application.application.id != manifest.application.id {
                return Err(Error::access_denied("bundle identity does not match launcher policy".into()));
            }

            if let Some(archetype_name) = application.application.archetype.as_ref() {
                let archetype = self
                    .archetypes
                    .get(archetype_name)
                    .ok_or_else(|| Error::invalid_argument("grant references unknown archetype".into()))?;

                maximum.extend(archetype);
            }

            maximum.extend(&application.grants);
        }

        let maximum_root = parse_rights(&maximum.root_rights)?;

        let requested_root = requested_root_rights(manifest)?;

        if !maximum_root.contains(requested_root) {
            return Err(Error::access_denied("application requested filesystem rights denied by policy".into()));
        }

        for requested in &manifest.permissions.capabilities {
            validate_capability(requested)?;

            if !maximum.capabilities.iter().any(|allowed| allowed == requested) {
                return Err(Error::access_denied(format!("capability {} denied by policy", requested,).into()));
            }
        }
        let mut capabilities = Vec::with_capacity(manifest.permissions.capabilities.len());

        for requested in &manifest.permissions.capabilities {
            validate_capability(requested)?;

            if !maximum.capabilities.iter().any(|allowed| allowed == requested) {
                return Err(Error::access_denied(format!("capability {} denied by policy", requested,).into()));
            }

            capabilities.push(resolve_capability(requested)?);
        }

        Ok(LaunchPolicy { root_rights: requested_root, cwd_rights: parse_rights(&maximum.cwd_rights)?, capabilities })
    }
}

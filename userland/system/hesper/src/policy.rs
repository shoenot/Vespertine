use alloc::collections::BTreeMap;

use vespertine_abi::CapabilityID;
use vespertine_abi::tag::{
    CAP_APP_TERMCTRL, CAP_FRAMEBUFFER, CAP_PROCMAN,
};
use vconfig::ConfigError;
use vconfig::manifest::AppManifest;
use vconfig::policy::{
    GrantFile,
    GrantSet,
    parse_archetype_file,
    parse_grant_file,
};
use vstd::fs::EntryKind;
use vstd::{NamespaceRights, prelude::*};

const ARCHETYPES_PATH: &str = "/System/Policy/archetypes.toml";

const GRANTS_PATH: &str = "/System/Policy/Grants";

#[derive(Debug)]
pub struct LaunchPolicy {
    pub namespace_rights: NamespaceRights,
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

fn read_text(path: &str) -> Result<String, Error> {
    File::open(&Path::new(path))?
        .read_to_string()
        .map_err(|error| error.context(format_args!("cannot read configuration '{path}'")))
}

fn config_error(error: ConfigError) -> Error {
    match error {
        ConfigError::Invalid(message) => Error::invalid_argument(message),
        ConfigError::Parse(message) => Error::invalid_encoding(message),
        ConfigError::NotFound(message) => Error::not_found(message),
    }
}

impl PolicyStore {
    pub fn load() -> Result<Self, Error> {
        let archetypes_text = read_text(ARCHETYPES_PATH)?;

        let archetypes = parse_archetype_file(&archetypes_text).map_err(config_error)?;

        if archetypes.version != 1 {
            return Err(Error::invalid_argument("unsupported launcher policy version".into()));
        }

        let grants_dir = Dir::open(&Path::new(GRANTS_PATH))?;
        let mut applications = BTreeMap::new();

        let context = |error: Error| error.context(format_args!("listing grants in '{GRANTS_PATH}'"));
        for entry in grants_dir.list().map_err(context)? {
            let entry = entry.map_err(context)?;
            if !matches!(entry.kind, EntryKind::File) {
                continue;
            }

            let Some(app_id) = entry.name.strip_suffix(".toml") else {
                continue;
            };

            let path = format!("{}/{}", GRANTS_PATH, entry.name,);

            let text = read_text(&path)?;

            let grant = parse_grant_file(&text, &path).map_err(config_error)?;

            if grant.application.id != app_id {
                return Err(Error::invalid_argument(format!("grant app ID does not match bundle app ID {}", grant.application.id,).into()));
            }

            if let Some(archetype) = grant.application.archetype.as_ref() {
                if !archetypes.archetype.contains_key(archetype) {
                    return Err(Error::invalid_argument(format!("unknown archetype {}", archetype,).into()));
                }
            }

            let app_id = grant.application.id.clone();

            if applications.insert(app_id.clone(), grant).is_some() {
                return Err(Error::invalid_argument(format!("duplicate policy for app ID {}", app_id).into()));
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
        "procman-list" => Ok(CapabilityPolicy { capability: CAP_PROCMAN, rights: AccessRights::READ | AccessRights::LIST }),
        "framebuffer" => Ok(CapabilityPolicy { capability: CAP_FRAMEBUFFER, rights: AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE }),
        _ => Err(Error::invalid_argument(format!("unknown capability {}", name,).into())),
    }
}

fn validate_capability(name: &str) -> Result<(), Error> { resolve_capability(name).map(|_| ()) }

impl PolicyStore {
    fn validate(&self) -> Result<(), Error> {
        parse_rights(&self.defaults.namespace_rights)?;
        parse_rights(&self.defaults.cwd_rights)?;

        for capability in &self.defaults.capabilities {
            validate_capability(capability)?;
        }

        for archetype in self.archetypes.values() {
            parse_rights(&archetype.namespace_rights)?;
            parse_rights(&archetype.cwd_rights)?;

            for capability in &archetype.capabilities {
                validate_capability(capability)?;
            }
        }

        for grant in self.applications.values() {
            parse_rights(&grant.grants.namespace_rights)?;
            parse_rights(&grant.grants.cwd_rights)?;

            for capability in &grant.grants.capabilities {
                validate_capability(capability)?;
            }
        }

        Ok(())
    }
}

fn requested_namespace_rights(manifest: &AppManifest) -> Result<NamespaceRights, Error> {
    let raw = match manifest.permissions.filesystem.as_str() {
        "read-only" => AccessRights::READ | AccessRights::TRAVERSE | AccessRights::LIST,
        "mutable" => AccessRights::READ |
            AccessRights::WRITE |
            AccessRights::CREATE |
            AccessRights::REMOVE |
            AccessRights::TRAVERSE |
            AccessRights::LIST,
        "full" => AccessRights::all(),
        _ => return Err(Error::invalid_argument("unknown filesystem permission request".into())),
    };
    Ok(NamespaceRights::from_access_rights(raw))
}

impl PolicyStore {
    pub fn resolve(&self, app_id: &str, manifest: &AppManifest) -> Result<LaunchPolicy, Error> {
        if manifest.application.id != app_id {
            return Err(Error::access_denied("manifest identity does not match launcher request".into()));
        }
        let mut maximum = self.defaults.clone();

        if let Some(application) = self.applications.get(app_id) {
            if application.application.id != manifest.application.id {
                return Err(Error::access_denied("grant identity does not match manifest identity".into()));
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

        let maximum_ns_rights = parse_rights(&maximum.namespace_rights)?;
        let requested_ns_rights = requested_namespace_rights(manifest)?;
        let requested_ns_rights_raw = requested_ns_rights.as_access_rights();

        if !maximum_ns_rights.contains(requested_ns_rights_raw) {
            return Err(Error::access_denied(format!(
                "launch policy denied namespace rights for '{app_id}': \
                 requested {requested_ns_rights_raw}, allowed {maximum_ns_rights}, missing {}",
                requested_ns_rights_raw.difference(maximum_ns_rights),
            )));
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

        Ok(LaunchPolicy { namespace_rights: requested_ns_rights, cwd_rights: parse_rights(&maximum.cwd_rights)?, capabilities })
    }

    pub fn app_ids(&self) -> impl Iterator<Item = &str> { self.applications.keys().map(|id| id.as_str()) }
}

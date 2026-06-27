extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Deserialize;
use toml::value::Datetime;
use vespertine_abi::app::hesper::{
    AppIoMode,
    AppIoModes,
};
use vespertine_std::fs::{
    File,
    Path,
};
use vespertine_std::hesper::{
    decode_io_mode_string,
    decode_io_modes_strings,
};
use vespertine_std::vreg::ResolvedApplication;
use vespertine_std::{
    Error,
    Read,
};

const REGISTRY_PATH: &str = "/System/Registry/Applications";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RegistryFile {
    version: u32,
    application: Vec<ApplicationRecord>,
    installation: InstallationMetadata,
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
pub struct AppManifest {
    pub application: AppMetadata,
    pub entrypoints: BTreeMap<String, EntrypointMetadata>,
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
pub struct InstallationMetadata {
    pub installed_ts: Datetime,
    pub updated_ts: Datetime,
}

#[derive(Debug, Clone)]
pub struct LaunchTarget {
    pub command: String,
    pub app_id: String,
    pub bundle: String,
    pub entrypoint: String,
}

pub struct RegisteredApplication {
    record: ApplicationRecord,
    installation: InstallationMetadata,
}

pub struct AppRegistry {
    apps: BTreeMap<String, RegisteredApplication>,
    aliases: BTreeMap<String, (String, String)>,
}

const REGISTRY_INDEX: &str = "/System/Registry/index.toml";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RegistryIndex {
    version: u32,
    applications: Vec<String>,
}

fn default_io_mode() -> String { String::from("text") }

fn read_text(path: &str) -> Result<String, Error> {
    File::open(&Path::new(path))?.read_to_string()
}

fn validate_component(value: &str, desc: &str) -> Result<(), Error> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') ||
    value.as_bytes().contains(&0) {
        return Err(Error::invalid_argument(format!("invalid {}", desc).into()));
    }

    if value.len() > 254 {
        return Err(Error::name_too_long(format!("{} is too long", desc).into()));
    }

    Ok(())
}

fn validate_app_id(id: &str) -> Result<(), Error> {
    if id.is_empty() || id.contains('/') || id.as_bytes().contains(&0) {
        return Err(Error::invalid_argument("invalid app ID".into()));
    }

    Ok(())
}

fn load_manifest(bundle: &str) -> Result<AppManifest, Error> {
    let manifest_path = format!("{}/manifest.toml", bundle);
    let text = read_text(&manifest_path)?;

    let manifest = toml::from_str::<AppManifest>(&text)
        .map_err(|error| Error::invalid_encoding(format!("invalid application manifest {}: {:?}", manifest_path, error).into()))?;

    validate_app_id(&manifest.application.id)?;

    if manifest.application.name.is_empty() {
        return Err(Error::invalid_argument(format!("manifest {} has empty display name", manifest_path).into()));
    }

    if manifest.entrypoints.is_empty() {
        return Err(Error::invalid_argument(format!("manifest {} has no entrypoints", manifest_path).into()));
    }

    for (name, entrypoint) in &manifest.entrypoints {
        validate_component(name, "entrypoint name")?;
        validate_component(&entrypoint.binary, "application binary name")?;

        let modes = decode_io_modes_strings(&entrypoint.modes)?;
        let default_mode = decode_io_mode_string(&entrypoint.default)?;

        if default_mode == AppIoMode::Any {
            return Err(Error::invalid_argument(format!("entrypoint {} default mode cannot be any", name).into()));
        }

        if !modes.contains_mode(default_mode) {
            return Err(Error::invalid_argument(format!("entrypoint {} default mode is not supported", name).into()));
        }
    }

    Ok(manifest)
}

fn select_entrypoint(manifest: &AppManifest, name: &str) -> Result<EntrypointMetadata, Error> {
    manifest.entrypoints.get(name).cloned()
        .ok_or_else(|| Error::not_found("application entrypoint was not found".into()))
}

fn validate_record(path: &str, app: &ApplicationRecord) -> Result<(), Error> {
    validate_app_id(&app.id)?;

    if app.default_entrypoint.is_empty() {
        return Err(Error::invalid_argument(format!("registry app {} has empty default entrypoint", app.id).into()));
    }

    if app.bundle.is_empty() || !app.bundle.starts_with("/Programs/") || !
    app.bundle.ends_with(".app") {
        return Err(Error::invalid_argument(format!("registry app {} has invalid bundle path", app.id).into()));
    }

    let manifest = load_manifest(&app.bundle)?;

    if manifest.application.id != app.id {
        return Err(Error::invalid_argument(format!("registry app ID {} does not match bundle manifest ID {}", app.id, manifest.application.id).into()));
    }

    if !manifest.entrypoints.contains_key(&app.default_entrypoint) {
        return Err(Error::invalid_argument(format!("registry app {} default entrypoint {} does not exist", app.id, app.default_entrypoint).into()));
    }

    for (alias, entrypoint) in &app.aliases {
        validate_component(alias, "application alias")?;

        if entrypoint.is_empty() {
            return Err(Error::invalid_argument(format!("alias {} in {} has empty entrypoint", alias, path).into()));
        }

        if !manifest.entrypoints.contains_key(entrypoint) {
            return Err(Error::invalid_argument(format!("registry alias {} in {} points to missing entrypoint {}", alias, path, entrypoint).into()));
        }
    }

    Ok(())
}

fn app_io(entrypoint: &EntrypointMetadata) -> Result<(AppIoMode, AppIoModes, AppIoMode), Error> {
    let input = decode_io_mode_string(&entrypoint.input)?;
    let modes = decode_io_modes_strings(&entrypoint.modes)?;
    let default_mode = decode_io_mode_string(&entrypoint.default)?;

    if default_mode == AppIoMode::Any {
        return Err(Error::invalid_argument("application default mode cannot be any".into()));
    }

    if !modes.contains_mode(default_mode) {
        return Err(Error::invalid_argument("application default mode is not supported".into()));
    }

    Ok((input, modes, default_mode))
}

impl AppRegistry {
    pub fn load() -> Result<Self, Error> {
        let index_text = read_text(REGISTRY_INDEX)?;
        let index = toml::from_str::<RegistryIndex>(&index_text)
            .map_err(|error| Error::invalid_encoding(format!("invalid app registry index: {:?}", error).into()))?;

        if index.version != 1 {
            return Err(Error::invalid_argument("unsupported app registry index version".into()));
        }

        let mut apps = BTreeMap::new();
        let mut aliases = BTreeMap::new();

        for app_id in index.applications {
            validate_app_id(&app_id)?;

            if apps.contains_key(&app_id) {
                return Err(Error::invalid_argument(format!("duplicate app ID {}", app_id).into()));
            }

            let path = format!("{}/{}.toml", REGISTRY_PATH, app_id);
            let text = read_text(&path)?;
            let file = toml::from_str::<RegistryFile>(&text)
                .map_err(|error| Error::invalid_encoding(format!("invalid app registry file {}: {:?}", path, error).into()))?;

            if file.version != 1 {
                return Err(Error::invalid_argument(format!("unsupported app registry version in {}", path).into()));
            }

            if file.application.len() != 1 {
                return Err(Error::invalid_argument(format!("registry file {} must contain exactly one application", path).into()));
            }

            for app in file.application {
                if app.id != app_id {
                    return Err(Error::invalid_argument(format!("registry file {} contains app ID {}, expected {}", path, app.id, app_id).into()));
                }

                validate_record(&path, &app)?;

                for (alias, entrypoint) in &app.aliases {
                    if aliases.contains_key(alias) || apps.contains_key(alias) {
                        return Err(Error::invalid_argument(format!("duplicate application name or alias {}", alias).into()));
                    }

                    aliases.insert(alias.clone(), (app.id.clone(), entrypoint.clone()));
                }

                apps.insert(app.id.clone(), RegisteredApplication {
                    record: app,
                    installation: file.installation.clone(),
                });
            }
        }

        Ok(Self { apps, aliases })
    }

    pub fn resolve_alias(&self, alias: &str) -> Result<LaunchTarget, Error> {
        let (app_id, entrypoint) = self.aliases.get(alias)
            .ok_or_else(|| Error::not_found("application alias was not found".into()))?;

        let app = self.apps.get(app_id)
            .ok_or_else(|| Error::invalid_argument("application alias points to missing app".into()))?;

        Ok(LaunchTarget {
            command: alias.into(),
            app_id: app.record.id.clone(),
            bundle: app.record.bundle.clone(),
            entrypoint: entrypoint.clone(),
        })
    }

    pub fn resolve_app(&self, app_id: &str) -> Result<LaunchTarget, Error> {
        let app = self.apps.get(app_id)
            .ok_or_else(|| Error::not_found("application was not found".into()))?;

        Ok(LaunchTarget {
            command: app_id.into(),
            app_id: app.record.id.clone(),
            bundle: app.record.bundle.clone(),
            entrypoint: app.record.default_entrypoint.clone(),
        })
    }

    pub fn resolve_target(&self, name: &str) -> Result<LaunchTarget, Error> {
        match self.resolve_alias(name) {
            Ok(target) => Ok(target),
            Err(_) => self.resolve_app(name),
        }
    }

    pub fn resolve(&self, name: &str) -> Result<ResolvedApplication, Error> {
        let target = self.resolve_target(name)?;
        self.resolve_target_metadata(target)
    }

    pub fn list(&self) -> Result<Vec<ResolvedApplication>, Error> {
        let mut entries = Vec::new();

        for app in self.apps.values() {
            let target = LaunchTarget {
                command: app.record.id.clone(),
                app_id: app.record.id.clone(),
                bundle: app.record.bundle.clone(),
                entrypoint: app.record.default_entrypoint.clone(),
            };

            entries.push(self.resolve_target_metadata(target)?);
        }

        Ok(entries)
    }

    fn resolve_target_metadata(&self, target: LaunchTarget) -> Result<ResolvedApplication, Error> {
        let app_record = self.apps.get(&target.app_id)
            .ok_or_else(|| Error::not_found("application not found".into()))?;

        let manifest = load_manifest(&target.bundle)?;
        if manifest.application.id != target.app_id {
            return Err(Error::access_denied("registry app ID does not match bundle manifest".into()));
        }

        let entrypoint = select_entrypoint(&manifest, &target.entrypoint)?;
        let (input, modes, default_mode) = app_io(&entrypoint)?;

        Ok(ResolvedApplication {
            command: target.command,
            app_id: target.app_id,
            bundle: target.bundle,
            entrypoint: target.entrypoint,
            binary: entrypoint.binary,
            input,
            modes,
            default_mode,
            display_name: manifest.application.name,
            installed_ts: app_record.installation.installed_ts.to_string(),
            updated_ts: app_record.installation.updated_ts.to_string(),
        })
    }
}

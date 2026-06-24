use vespertine_abi::AccessRights;
use vespertine_std::Error;

use crate::meta::AppManifest;

#[derive(Debug, Clone, Copy)]
pub struct LaunchPolicy {
    pub root_rights: AccessRights,
    pub cwd_rights: AccessRights,
}

fn read_only_root_rights() -> AccessRights { AccessRights::READ | AccessRights::TRAVERSE | AccessRights::LIST }

fn mutable_root_rights() -> AccessRights { read_only_root_rights() | AccessRights::WRITE | AccessRights::CREATE | AccessRights::REMOVE }

fn standard_cwd_rights() -> AccessRights { AccessRights::TRAVERSE | AccessRights::LIST }

fn application_may_mutate(bundle_name: &str, application_id: &str) -> bool {
    matches!((bundle_name, application_id), ("ns", "os.vespertine.ns") | ("kilo", "org.antirez.kilo"))
}

pub fn launch_policy(bundle_name: &str, manifest: &AppManifest) -> Result<LaunchPolicy, Error> {
    let root_rights =
        match manifest.permissions.filesystem.as_str() {
            "read-only" => read_only_root_rights(),
            "mutable" => {
                if !application_may_mutate(bundle_name, &manifest.application.id) {
                    return Err(Error::access_denied("application is not permitted mutable filesystem access".into()));
                }
                mutable_root_rights()
            },
            _ => return Err(Error::invalid_argument("unknown filesystem permission request".into())),
        };
    Ok(LaunchPolicy { root_rights, cwd_rights: standard_cwd_rights() })
}


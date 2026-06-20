use alloc::{format, string::String};
use vespertine_std::{
    Error, Read,
    fs::{File, Path},
};

#[derive(Debug, serde::Deserialize)]
pub struct AppManifest {
    pub application: AppMetadata,
    pub io: IoMetadata,
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

pub fn get_metadata(name: &str) -> Result<AppManifest, Error> {
    let app_path = format!("/Programs/{}.app/manifest.toml", name);
    let manifest_file = File::open(&Path::new(app_path.as_str()))?;
    let manifest_str = manifest_file.read_to_string()?;

    let manifest = toml::from_str::<AppManifest>(manifest_str.as_str()).map_err(|e| {
        Error::invalid_encoding(format!("could not parse file into toml: {:?}", e).into())
    })?;

    Ok(manifest)
}

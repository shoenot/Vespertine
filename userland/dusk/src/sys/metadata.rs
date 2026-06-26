use vespertine_abi::app::hesper::{
    AppIoMode,
    AppIoModes,
    HESPER_STATUS_INVALID_REQUEST,
    HESPER_STATUS_NOT_FOUND,
    HESPER_STATUS_OK,
};
use vespertine_std::hesper::Launcher;
use vespertine_std::Error;

use crate::sys::ShellResult;

#[derive(Debug, Clone, Copy)]
pub(super) struct ProgramMetadata {
    pub input: AppIoMode,
    pub modes: AppIoModes,
    pub default_mode: AppIoMode,
}

pub(super) fn load_program_metadata(name: &str) -> Result<ProgramMetadata, ShellResult> {
    let mut launcher = Launcher::connect().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;
    let response = launcher.metadata(name).map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

    match response.status {
        HESPER_STATUS_OK => {
            if response.default_mode == AppIoMode::Any {
                return Err(ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("application manifest cannot declare default mode = any".into()),
                ));
            }

            if !response.modes.contains_mode(response.default_mode) {
                return Err(ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("application manifest default mode is not supported".into()),
                ));
            }

            Ok(ProgramMetadata { input: response.input, modes: response.modes, default_mode: response.default_mode })
        },
        HESPER_STATUS_NOT_FOUND => Err(ShellResult::NotFound(name.into())),
        HESPER_STATUS_INVALID_REQUEST => {
            Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application bundle contains an invalid manifest".into())))
        },
        _ => Err(ShellResult::FailedToLaunch(name.into(), Error::unknown("Hesper failed to return application metadata".into()))),
    }
}

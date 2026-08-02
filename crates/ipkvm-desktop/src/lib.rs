mod app;
mod fonts;
mod frame;
mod probe;
mod render;
mod session;
mod state;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("desktop gui failed: {0}")]
    Gui(String),
}

pub fn run() -> Result<(), DesktopError> {
    app::run().map_err(|error| DesktopError::Gui(error.to_string()))
}

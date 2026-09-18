use crate::physics::PhysicsConfig;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DirectionSwitching {
    Disabled,
    #[default]
    DragRelease,
    Drag,
    Hover,
}

/// Runtime options shared by the desktop and web frontends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PetConfig {
    pub allow_interact: bool,
    pub allow_sit: bool,
    pub allow_sleep: bool,
    pub allow_special: bool,
    pub allow_walk: bool,
    pub activity_level: u8,
    pub direction_switching: DirectionSwitching,
    pub walk_speed: f32,
    pub physics: PhysicsConfig,
    pub opacity_normal: f32,
    pub opacity_dim: f32,
    pub display_scale: f32,
    pub transition_duration: f32,
}

impl Default for PetConfig {
    fn default() -> Self {
        Self {
            allow_interact: true,
            allow_sit: true,
            allow_sleep: false,
            allow_special: true,
            allow_walk: true,
            activity_level: 4,
            direction_switching: DirectionSwitching::default(),
            walk_speed: 30.0,
            physics: PhysicsConfig::default(),
            opacity_normal: 1.0,
            opacity_dim: 0.75,
            display_scale: 1.0,
            transition_duration: 0.3,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl PetConfig {
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(text)?)
    }

    pub fn from_json_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json(&text)
    }
}

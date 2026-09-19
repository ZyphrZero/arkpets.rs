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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EasingFunction {
    Linear,
    EaseOutSine,
    #[default]
    EaseOutCubic,
    EaseOutQuint,
}

/// Original Ark-Pets presets. Configurations still store seconds to retain custom values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransitionPreset {
    Disabled,
    Fast,
    #[default]
    Standard,
    Slow,
}

impl TransitionPreset {
    pub const ALL: [Self; 4] = [Self::Disabled, Self::Fast, Self::Standard, Self::Slow];

    pub const fn seconds(self) -> f32 {
        match self {
            Self::Disabled => 0.0,
            Self::Fast => 0.1,
            Self::Standard => 0.3,
            Self::Slow => 0.6,
        }
    }

    /// Match exactly; a nearby custom duration must not be rounded to a preset.
    pub fn from_seconds(seconds: f32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.seconds() == seconds)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutlineCondition {
    Never,
    #[default]
    Dragging,
    Pressing,
    Focused,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutlineSettings {
    pub condition: OutlineCondition,
    pub controlled_condition: OutlineCondition,
    pub color: [u8; 4],
    pub controlled_color: [u8; 4],
    pub width: f32,
}

impl Default for OutlineSettings {
    fn default() -> Self {
        Self {
            condition: OutlineCondition::Dragging,
            controlled_condition: OutlineCondition::Focused,
            color: [255, 255, 0, 255],
            controlled_color: [255, 187, 0, 255],
            width: 2.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RenderSettings {
    pub outline: OutlineSettings,
    pub shadow_opacity: f32,
    pub high_quality: bool,
    pub mipmaps: bool,
    pub canvas: CanvasSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            outline: OutlineSettings::default(),
            shadow_opacity: 187.0 / 255.0,
            high_quality: true,
            mipmaps: true,
            canvas: CanvasSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CanvasSettings {
    pub coverage: f32,
    pub sampling_interval: u32,
}

impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            coverage: 0.8,
            sampling_interval: 4,
        }
    }
}

/// Runtime options shared by the desktop and web frontends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PetConfig {
    /// Controls triggered/automatic Interact animations, independently of dragging.
    pub allow_interact: bool,
    pub allow_drag: bool,
    pub allow_sit: bool,
    pub allow_sleep: bool,
    pub allow_special: bool,
    pub allow_walk: bool,
    pub activity_level: u8,
    pub direction_switching: DirectionSwitching,
    pub walk_speed: f32,
    pub physics: PhysicsConfig,
    pub rendering: RenderSettings,
    pub opacity_normal: f32,
    pub opacity_dim: f32,
    #[serde(alias = "opacityTransitionDuration")]
    pub property_transition_duration: f32,
    #[serde(alias = "opacityTransition")]
    pub easing: EasingFunction,
    pub display_scale: f32,
    /// Read the old Rust name as animation mixing, never as property smoothing.
    #[serde(alias = "transitionDuration")]
    pub animation_mix_duration: f32,
}

impl Default for PetConfig {
    fn default() -> Self {
        Self {
            allow_interact: true,
            allow_drag: true,
            allow_sit: true,
            allow_sleep: false,
            allow_special: true,
            allow_walk: true,
            activity_level: 4,
            direction_switching: DirectionSwitching::default(),
            walk_speed: 30.0,
            physics: PhysicsConfig::default(),
            rendering: RenderSettings::default(),
            opacity_normal: 1.0,
            opacity_dim: 0.75,
            property_transition_duration: TransitionPreset::Standard.seconds(),
            easing: EasingFunction::default(),
            display_scale: 1.0,
            animation_mix_duration: TransitionPreset::Standard.seconds(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_presets_match_original_durations_without_rounding_custom_values() {
        for (preset, seconds) in TransitionPreset::ALL.into_iter().zip([0.0, 0.1, 0.3, 0.6]) {
            assert_eq!(preset.seconds(), seconds);
            assert_eq!(TransitionPreset::from_seconds(seconds), Some(preset));
        }
        for seconds in [0.137, f32::from_bits(0.3_f32.to_bits() + 1)] {
            assert_eq!(TransitionPreset::from_seconds(seconds), None);
        }
    }
}

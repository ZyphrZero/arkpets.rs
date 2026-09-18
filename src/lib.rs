//! Ark-Pets behavior and native Spine 3.8 runtime.
//!
//! GUI and window integration live in the desktop binary. The types here model
//! the state that both the original desktop client
//! and the web implementation need: model assets, animation choices, physics,
//! dragging, and persistence.

pub mod animation;
pub mod character;
pub mod config;
pub mod interaction;
pub mod model;
pub mod physics;
pub mod runtime;
#[cfg(feature = "spine")]
pub mod spine;

pub use animation::{AnimationMachine, AnimationState, Direction};
pub use character::{CharacterSnapshot, CharacterState};
pub use config::{DirectionSwitching, PetConfig};
pub use model::{CharacterModel, ModelCatalog, ModelError};
pub use physics::{Bounds, PhysicsConfig, PhysicsState, Vec2};
pub use runtime::{CommandError, CommandSender, PetCommand, PetEvent, PetRuntime, SubmitError};

use crate::{
    animation::{AnimationMachine, AnimationState},
    config::PetConfig,
    model::{CharacterModel, ModelError},
    physics::{Bounds, PhysicsState, Vec2},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSnapshot {
    pub model: CharacterModel,
    pub animation: AnimationState,
    pub physics: PhysicsState,
    pub hidden: bool,
}

#[derive(Debug, Error)]
pub enum CharacterError {
    #[error("invalid snapshot JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model cannot be loaded: {0}")]
    Model(#[from] ModelError),
    #[error("cannot write snapshot {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

/// Mutable state for one pet. A frontend owns one instance and feeds it
/// pointer events and frame deltas; a renderer reads the public state each frame.
pub struct CharacterState {
    model: CharacterModel,
    config: PetConfig,
    animation: AnimationMachine,
    physics: PhysicsState,
    size: Vec2,
    hidden: bool,
}

impl CharacterState {
    pub fn new(model: CharacterModel, config: PetConfig, size: Vec2, seed: u64) -> Self {
        let animations = if model.animations.is_empty() {
            vec!["Relax", "Interact", "Move", "Sit", "Sleep"]
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            model.animations.clone()
        };
        let mut animation = AnimationMachine::with_seed(animations, seed);
        animation.configure(&config);
        Self {
            model,
            config,
            animation,
            physics: PhysicsState::default(),
            size,
            hidden: false,
        }
    }

    pub fn model(&self) -> &CharacterModel {
        &self.model
    }
    pub fn config(&self) -> &PetConfig {
        &self.config
    }
    pub fn animation(&self) -> &AnimationState {
        &self.animation.state
    }
    pub fn available_animations(&self) -> &[String] {
        self.animation.available()
    }
    pub fn physics(&self) -> &PhysicsState {
        &self.physics
    }
    pub fn position(&self) -> Vec2 {
        self.physics.position
    }
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
    pub fn configure(&mut self, config: PetConfig) {
        if self.config.allow_sit != config.allow_sit
            || self.config.allow_sleep != config.allow_sleep
            || self.config.allow_walk != config.allow_walk
            || self.config.allow_special != config.allow_special
            || self.config.activity_level != config.activity_level
            || self.config.allow_interact != config.allow_interact
        {
            self.animation.configure(&config);
        }
        if !config.allow_drag {
            self.physics.end_drag();
        }
        self.config = config;
    }

    pub fn resize(&mut self, size: Vec2) {
        self.size = size;
    }
    pub fn set_position(&mut self, position: Vec2, bounds: Bounds) {
        self.physics.position = Vec2::new(
            position.x.clamp(0.0, bounds.max_position(self.size).x),
            position.y.clamp(0.0, bounds.max_position(self.size).y),
        );
    }

    pub fn tick(&mut self, dt: f32, bounds: Bounds) {
        if self.hidden {
            return;
        }
        self.animation.tick(dt);
        let walking = if self.config.allow_walk && self.animation.state.animation == "Move" {
            Some((self.animation.state.direction, self.config.walk_speed))
        } else {
            None
        };
        self.physics
            .step(dt, bounds, self.size, self.config.physics, walking);
        if walking.is_some() && !self.physics.dragging {
            if self.physics.position.x <= 0.0 {
                self.animation.set_direction(crate::Direction::Right);
            } else if self.physics.position.x >= bounds.max_position(self.size).x {
                self.animation.set_direction(crate::Direction::Left);
            }
        }
    }

    pub fn complete_animation(&mut self) {
        self.animation.complete();
    }

    pub fn set_direction(&mut self, direction: crate::Direction) {
        self.animation.set_direction(direction);
    }

    pub fn play_animation(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        if (name == "Sit" && !self.config.allow_sit)
            || (name == "Sleep" && !self.config.allow_sleep)
            || (name == "Move" && !self.config.allow_walk)
            || (name == "Special" && !self.config.allow_special)
        {
            return false;
        }
        self.animation.play(name)
    }

    pub fn click(&mut self) -> bool {
        if !self.config.allow_interact {
            return false;
        }
        self.animation.interact()
    }

    pub fn start_drag(&mut self, cursor: Vec2) {
        if !self.config.allow_drag {
            return;
        }
        self.physics.start_drag(cursor);
        if self.config.allow_interact {
            let _ = self.animation.play("Relax");
        }
    }

    pub fn drag_to(&mut self, cursor: Vec2, dt: f32, bounds: Bounds) {
        self.physics.drag_to(
            cursor,
            dt,
            bounds,
            self.size,
            self.config.physics.max_velocity,
        );
    }

    pub fn end_drag(&mut self) {
        self.physics.end_drag();
    }
    pub fn hide(&mut self) {
        self.hidden = true;
    }
    pub fn show(&mut self) {
        self.hidden = false;
    }

    pub fn switch_model(&mut self, model: CharacterModel, seed: u64) {
        let config = self.config.clone();
        let size = self.size;
        let position = self.physics.position;
        *self = Self::new(model, config, size, seed);
        self.physics.position = position;
    }

    pub fn snapshot(&self) -> CharacterSnapshot {
        CharacterSnapshot {
            model: self.model.clone(),
            animation: self.animation.state.clone(),
            physics: self.physics,
            hidden: self.hidden,
        }
    }

    pub fn from_snapshot(
        snapshot: CharacterSnapshot,
        config: PetConfig,
        size: Vec2,
        seed: u64,
    ) -> Self {
        let mut state = Self::new(snapshot.model, config, size, seed);
        state.physics = snapshot.physics;
        state.physics.end_drag();
        if state.animation.play(&snapshot.animation.animation) {
            state.animation.state = snapshot.animation;
        }
        state.hidden = snapshot.hidden;
        state
    }

    pub fn to_json(&self) -> Result<String, CharacterError> {
        Ok(serde_json::to_string_pretty(&self.snapshot())?)
    }

    pub fn save_json_file(&self, path: impl AsRef<Path>) -> Result<(), CharacterError> {
        let path = path.as_ref();
        fs::write(path, self.to_json()?).map_err(|source| CharacterError::Write {
            path: path.to_owned(),
            source,
        })
    }

    pub fn from_json(
        text: &str,
        config: PetConfig,
        size: Vec2,
        seed: u64,
    ) -> Result<Self, CharacterError> {
        Ok(Self::from_snapshot(
            serde_json::from_str(text)?,
            config,
            size,
            seed,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> CharacterModel {
        CharacterModel {
            id: "demo".into(),
            name: "Demo".into(),
            skeleton: "a.skel".into(),
            atlas: "a.atlas".into(),
            texture: "a.png".into(),
            animations: vec!["Relax", "Interact", "Move"]
                .into_iter()
                .map(String::from)
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn click_and_drag_match_web_behavior() {
        let mut character =
            CharacterState::new(model(), PetConfig::default(), Vec2::new(150.0, 150.0), 1);
        assert!(character.click());
        assert_eq!(character.animation().animation, "Interact");
        character.start_drag(Vec2::new(10.0, 10.0));
        character.drag_to(Vec2::new(100.0, 100.0), 0.1, Bounds::new(500.0, 400.0));
        character.end_drag();
        assert_eq!(character.position(), Vec2::new(90.0, 90.0));
        assert_eq!(character.physics().velocity, Vec2::new(900.0, 900.0));
    }

    #[test]
    fn interaction_animation_and_dragging_are_independent() {
        let mut config = PetConfig {
            allow_interact: false,
            ..Default::default()
        };
        let mut character = CharacterState::new(model(), config.clone(), Vec2::new(20.0, 20.0), 1);
        assert!(character.play_animation("Move"));
        assert!(!character.click());
        character.start_drag(Vec2::ZERO);
        assert!(character.physics().dragging);
        assert_eq!(character.animation().animation, "Move");
        character.drag_to(Vec2::new(30.0, 20.0), 0.1, Bounds::new(200.0, 200.0));
        assert_eq!(character.position(), Vec2::new(30.0, 20.0));
        config.allow_drag = false;
        config.allow_interact = true;
        character.configure(config);
        assert!(!character.physics().dragging);
        character.start_drag(Vec2::ZERO);
        assert!(!character.physics().dragging);
        assert!(character.click());
        assert_eq!(character.animation().animation, "Interact");
    }

    #[test]
    fn snapshot_round_trip() {
        let character =
            CharacterState::new(model(), PetConfig::default(), Vec2::new(150.0, 150.0), 2);
        let json = character.to_json().unwrap();
        let restored =
            CharacterState::from_json(&json, PetConfig::default(), Vec2::new(150.0, 150.0), 2)
                .unwrap();
        assert_eq!(restored.model().id, "demo");
        assert_eq!(restored.animation().animation, "Relax");
    }

    #[test]
    fn walking_turns_at_the_viewport_boundary() {
        let mut character =
            CharacterState::new(model(), PetConfig::default(), Vec2::new(150.0, 150.0), 2);
        let bounds = Bounds::new(300.0, 200.0);
        character.set_position(Vec2::new(150.0, 50.0), bounds);
        character.play_animation("Move");
        character.tick(1.0 / 60.0, bounds);
        assert_eq!(character.animation().direction, crate::Direction::Left);
        character.tick(1.0 / 60.0, bounds);
        assert!(character.position().x < 150.0);
    }

    #[test]
    fn walking_speed_is_live_and_independent_of_animation_playback() {
        let mut character =
            CharacterState::new(model(), PetConfig::default(), Vec2::new(100.0, 100.0), 1);
        let bounds = Bounds::new(1000.0, 500.0);
        character.set_position(Vec2::new(300.0, 400.0), bounds);
        character.play_animation("Move");
        for speed in [0.0, 30.0, 200.0] {
            character.configure(PetConfig {
                walk_speed: speed,
                ..Default::default()
            });
            let before = character.position().x;
            character.tick(0.1, bounds);
            assert!((character.position().x - before - speed * 0.1).abs() < 0.001);
        }
        assert!((character.animation().timestamp - 0.3).abs() < 0.001);
    }

    #[test]
    fn snapshot_does_not_restore_an_unfinished_pointer_drag() {
        let mut character =
            CharacterState::new(model(), PetConfig::default(), Vec2::new(150.0, 150.0), 2);
        character.start_drag(Vec2::new(10.0, 10.0));
        let restored = CharacterState::from_snapshot(
            character.snapshot(),
            PetConfig::default(),
            Vec2::new(150.0, 150.0),
            2,
        );
        assert!(!restored.physics().dragging);
    }
}

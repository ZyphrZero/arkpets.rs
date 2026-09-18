use std::sync::{
    mpsc::{self, Receiver, SyncSender, TrySendError},
    Arc,
};

use thiserror::Error;

use crate::{Bounds, CharacterState, Direction, PetConfig};

pub const COMMAND_QUEUE_CAPACITY: usize = 64;
pub const COMMANDS_PER_FRAME: usize = 16;
const MAX_ANIMATION_NAME_BYTES: usize = 256;

/// Commands are independent of the GUI and transport used to deliver them.
#[derive(Debug, Clone)]
pub enum PetCommand {
    PlayAnimation(String),
    SetDirection(Direction),
    Configure(PetConfig),
    SetPaused(bool),
    SetAutomatic(bool),
    Interact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    PlayAnimation,
    SetDirection,
    Configure,
    SetPaused,
    SetAutomatic,
    Interact,
}

impl PetCommand {
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::PlayAnimation(_) => CommandKind::PlayAnimation,
            Self::SetDirection(_) => CommandKind::SetDirection,
            Self::Configure(_) => CommandKind::Configure,
            Self::SetPaused(_) => CommandKind::SetPaused,
            Self::SetAutomatic(_) => CommandKind::SetAutomatic,
            Self::Interact => CommandKind::Interact,
        }
    }

    fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::PlayAnimation(name)
                if name.is_empty() || name.len() > MAX_ANIMATION_NAME_BYTES =>
            {
                Err(CommandError::InvalidAnimationName)
            }
            Self::Configure(config) => {
                if config.activity_level > 16 {
                    return Err(CommandError::InvalidConfig("activity_level"));
                }
                for (field, value) in [
                    ("walk_speed", config.walk_speed),
                    ("physics.gravity", config.physics.gravity),
                    ("physics.max_velocity.x", config.physics.max_velocity.x),
                    ("physics.max_velocity.y", config.physics.max_velocity.y),
                    ("physics.min_velocity", config.physics.min_velocity),
                    ("transition_duration", config.transition_duration),
                ] {
                    if !value.is_finite() || value < 0.0 {
                        return Err(CommandError::InvalidConfig(field));
                    }
                }
                for (field, value) in [
                    ("physics.drag", config.physics.drag),
                    ("physics.bounce_damping", config.physics.bounce_damping),
                    ("opacity_normal", config.opacity_normal),
                    ("opacity_dim", config.opacity_dim),
                ] {
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(CommandError::InvalidConfig(field));
                    }
                }
                if !config.display_scale.is_finite() || config.display_scale <= 0.0 {
                    return Err(CommandError::InvalidConfig("display_scale"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("animation name must contain 1 to 256 UTF-8 bytes")]
    InvalidAnimationName,
    #[error("animation is unavailable or disabled")]
    AnimationUnavailable,
    #[error("interaction is unavailable or disabled")]
    InteractionUnavailable,
    #[error("invalid config field: {0}")]
    InvalidConfig(&'static str),
}

#[derive(Debug, Error)]
pub enum SubmitError {
    #[error(transparent)]
    Invalid(#[from] CommandError),
    #[error("pet command queue is full")]
    Full(PetCommand),
    #[error("pet runtime has stopped")]
    Closed(PetCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetEvent {
    pub command: CommandKind,
    pub result: Result<(), CommandError>,
}

/// Cloneable ingress for adapters. Sending never waits for the render thread.
#[derive(Clone)]
pub struct CommandSender {
    sender: SyncSender<PetCommand>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CommandSender {
    /// The callback should schedule a frame and return immediately.
    pub fn with_waker(mut self, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.wake = Some(wake);
        self
    }

    pub fn try_send(&self, command: PetCommand) -> Result<(), SubmitError> {
        command.validate()?;
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(command) => SubmitError::Full(command),
            TrySendError::Disconnected(command) => SubmitError::Closed(command),
        })?;
        if let Some(wake) = &self.wake {
            wake();
        }
        Ok(())
    }
}

/// One owner of pet state; adapters submit commands and the frontend drives time.
pub struct PetRuntime {
    character: CharacterState,
    paused: bool,
    automatic: bool,
    sender: SyncSender<PetCommand>,
    receiver: Receiver<PetCommand>,
}

impl PetRuntime {
    pub fn new(character: CharacterState) -> Self {
        let (sender, receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        Self {
            character,
            paused: false,
            automatic: true,
            sender,
            receiver,
        }
    }

    pub fn command_sender(&self) -> CommandSender {
        CommandSender {
            sender: self.sender.clone(),
            wake: None,
        }
    }

    pub fn character(&self) -> &CharacterState {
        &self.character
    }

    pub fn character_mut(&mut self) -> &mut CharacterState {
        &mut self.character
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn automatic(&self) -> bool {
        self.automatic
    }

    /// Synchronous UI commands use the same validation as queued adapter commands.
    pub fn apply(&mut self, command: PetCommand) -> Result<(), CommandError> {
        command.validate()?;
        match command {
            PetCommand::PlayAnimation(name) => {
                if !self.character.play_animation(name) {
                    return Err(CommandError::AnimationUnavailable);
                }
            }
            PetCommand::SetDirection(direction) => self.character.set_direction(direction),
            PetCommand::Configure(config) => self.character.configure(config),
            PetCommand::SetPaused(paused) => self.paused = paused,
            PetCommand::SetAutomatic(automatic) => self.automatic = automatic,
            PetCommand::Interact => {
                if !self.character.click() {
                    return Err(CommandError::InteractionUnavailable);
                }
            }
        }
        Ok(())
    }

    /// The caller consumes outcomes immediately, so a slow observer cannot grow a queue.
    pub fn drain_commands(&mut self, mut emit: impl FnMut(PetEvent)) -> usize {
        let mut count = 0;
        while count < COMMANDS_PER_FRAME {
            let Ok(command) = self.receiver.try_recv() else {
                break;
            };
            let kind = command.kind();
            let result = self.apply(command);
            emit(PetEvent {
                command: kind,
                result,
            });
            count += 1;
        }
        count
    }

    pub fn tick(&mut self, dt: f32, bounds: Bounds) {
        if !self.paused {
            self.character.tick(dt, bounds);
        }
    }

    pub fn animation_completed(&mut self) {
        if !self.paused
            && self.automatic
            && !self.character.is_hidden()
            && !self.character.physics().dragging
        {
            self.character.complete_animation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CharacterModel, Vec2};

    fn runtime() -> PetRuntime {
        PetRuntime::new(CharacterState::new(
            CharacterModel {
                animations: vec!["Relax".into(), "Interact".into(), "Move".into()],
                ..Default::default()
            },
            PetConfig::default(),
            Vec2::new(100.0, 100.0),
            1,
        ))
    }

    #[test]
    fn queue_is_bounded_and_frame_work_is_limited() {
        let mut runtime = runtime();
        let sender = runtime.command_sender();
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            sender.try_send(PetCommand::SetPaused(true)).unwrap();
        }
        assert!(matches!(
            sender.try_send(PetCommand::Interact),
            Err(SubmitError::Full(PetCommand::Interact))
        ));
        let mut events = Vec::new();
        assert_eq!(
            runtime.drain_commands(|event| events.push(event)),
            COMMANDS_PER_FRAME
        );
        assert_eq!(events.len(), COMMANDS_PER_FRAME);
        assert!(events.iter().all(|event| event.result.is_ok()));
        assert!(runtime.paused());
        sender.try_send(PetCommand::SetPaused(false)).unwrap();
        drop(runtime);
        assert!(matches!(
            sender.try_send(PetCommand::Interact),
            Err(SubmitError::Closed(PetCommand::Interact))
        ));
    }

    #[test]
    fn queued_commands_keep_order_and_report_rejected_actions() {
        let mut runtime = runtime();
        let sender = runtime.command_sender();
        let config = PetConfig {
            allow_walk: false,
            ..PetConfig::default()
        };
        sender.try_send(PetCommand::Configure(config)).unwrap();
        sender
            .try_send(PetCommand::PlayAnimation("Move".into()))
            .unwrap();
        sender.try_send(PetCommand::Interact).unwrap();
        let mut events = Vec::new();
        assert_eq!(runtime.drain_commands(|event| events.push(event)), 3);
        assert_eq!(events[0].command, CommandKind::Configure);
        assert!(events[0].result.is_ok());
        assert_eq!(events[1].result, Err(CommandError::AnimationUnavailable));
        assert_eq!(events[2].command, CommandKind::Interact);
        assert!(events[2].result.is_ok());
        assert_eq!(runtime.character().animation().animation, "Interact");
    }

    #[test]
    fn invalid_commands_never_enter_the_queue_or_mutate_state() {
        let mut runtime = runtime();
        let sender = runtime.command_sender();
        assert!(matches!(
            sender.try_send(PetCommand::Configure(PetConfig {
                activity_level: 17,
                ..Default::default()
            })),
            Err(SubmitError::Invalid(CommandError::InvalidConfig(
                "activity_level"
            )))
        ));
        assert!(matches!(
            sender.try_send(PetCommand::PlayAnimation("x".repeat(257))),
            Err(SubmitError::Invalid(CommandError::InvalidAnimationName))
        ));
        let mut config = PetConfig::default();
        config.physics.max_velocity.x = f32::NAN;
        assert_eq!(
            runtime.apply(PetCommand::Configure(config)),
            Err(CommandError::InvalidConfig("physics.max_velocity.x"))
        );
        assert_eq!(
            runtime.drain_commands(|_| panic!("invalid command queued")),
            0
        );
        runtime.tick(1.0 / 60.0, Bounds::new(500.0, 500.0));
        assert!(runtime.character().position().y.is_finite());
    }

    #[test]
    fn pause_freezes_simulation_and_manual_animation_does_not_auto_transition() {
        let mut runtime = runtime();
        let bounds = Bounds::new(500.0, 500.0);
        runtime
            .apply(PetCommand::PlayAnimation("Move".into()))
            .unwrap();
        runtime.apply(PetCommand::SetAutomatic(false)).unwrap();
        runtime.apply(PetCommand::SetPaused(true)).unwrap();
        runtime.animation_completed();
        runtime.tick(0.05, bounds);
        assert_eq!(runtime.character().position(), Vec2::ZERO);
        assert_eq!(runtime.character().animation().timestamp, 0.0);
        runtime.apply(PetCommand::SetPaused(false)).unwrap();
        runtime.animation_completed();
        runtime.tick(0.05, bounds);
        assert!(runtime.character().position().x > 0.0);
        assert!(runtime.character().position().y > 0.0);
        assert_eq!(runtime.character().animation().animation, "Move");
    }

    #[test]
    fn sender_wakes_the_frontend_only_after_successful_submission() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let runtime = runtime();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let sender = runtime.command_sender().with_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            sender.try_send(PetCommand::Interact).unwrap();
        }
        assert!(sender.try_send(PetCommand::Interact).is_err());
        assert!(sender
            .try_send(PetCommand::PlayAnimation(String::new()))
            .is_err());
        drop(runtime);
        assert!(sender.try_send(PetCommand::Interact).is_err());
        assert_eq!(wakes.load(Ordering::Relaxed), COMMAND_QUEUE_CAPACITY);
    }
}

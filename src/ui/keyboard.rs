//! Focused-window keyboard controls; no global hooks or second animation clock.
use arkpets::{animation, interaction::WalkControl, CharacterState, Direction};
use eframe::egui::{self, Context, Id, Key, ViewportId};

pub fn target_id(viewport: ViewportId) -> Id {
    Id::new(("pet-keyboard-control", viewport))
}

pub fn focus_response(response: &egui::Response) {
    let id = target_id(response.ctx.viewport_id());
    if response.clicked() {
        response.ctx.memory_mut(|memory| memory.request_focus(id));
    }
    response.ctx.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        )
    });
}

#[derive(Default)]
pub struct Control {
    owner: Option<ViewportId>,
    resting: Option<String>,
    walking_animation: Option<String>,
    walk: Option<WalkControl>,
}

impl Control {
    pub fn owner(&self) -> Option<ViewportId> {
        self.owner
    }
    pub fn walking(&self) -> Option<WalkControl> {
        self.walk
    }

    pub fn resting_action(&self) -> Option<&str> {
        self.resting.as_deref()
    }

    /// Mouse/API action changes while a movement key is held select the action
    /// to resume. Do not install an attack track or restart the current stride.
    pub fn prioritize_walk(&mut self, character: &mut CharacterState) -> bool {
        let (Some(control), Some(movement)) = (self.walk, self.walking_animation.as_ref()) else {
            return false;
        };
        if character.physics().dragging
            || !character.config().allow_walk
            || !character.available_animations().contains(movement)
        {
            return false;
        }
        character.set_direction(control.direction);
        if character.animation().animation == *movement {
            return false;
        }
        self.resting = Some(character.animation().animation.clone());
        character.play_animation(movement.clone())
    }

    pub fn cancel(&mut self, character: &mut CharacterState) {
        self.release(character);
        self.owner = None;
    }

    fn release(&mut self, character: &mut CharacterState) {
        if self.walking_animation.as_deref() == Some(character.animation().animation.as_str()) {
            if let Some(resting) = self.resting.take() {
                if !character.play_animation(resting) {
                    // The saved action may have been disabled while walking.
                    character.play_idle();
                }
            }
        }
        self.resting = None;
        self.walking_animation = None;
        self.walk = None;
    }

    pub fn update(
        &mut self,
        ctx: &Context,
        character: &mut CharacterState,
        enabled: bool,
        viewport_enabled: bool,
    ) {
        if !enabled || character.physics().dragging || character.is_hidden() {
            self.cancel(character);
            return;
        }
        if self.owner.is_some_and(|owner| {
            ctx.input(|input| {
                input.raw.viewports.get(&owner).is_none_or(|viewport| {
                    viewport.focused == Some(false) || viewport.visible() == Some(false)
                })
            })
        }) {
            self.cancel(character);
        }
        let viewport = ctx.viewport_id();
        let focused = ctx.input(|input| input.focused && input.viewport().focused != Some(false));
        let widget_allows =
            ctx.memory(|memory| memory.focused().is_none_or(|id| id == target_id(viewport)));
        if !viewport_enabled || !focused || !widget_allows || ctx.text_edit_focused() {
            if self.owner == Some(viewport) {
                self.cancel(character);
            }
            return;
        }
        let (left, right, fast, cycle) = ctx.input(|input| {
            let mut cycle = 0_i32;
            if !input.modifiers.alt && !input.modifiers.mac_cmd {
                for event in &input.events {
                    match event {
                        egui::Event::Key {
                            key: Key::ArrowUp,
                            pressed: true,
                            repeat: false,
                            ..
                        } => cycle -= 1,
                        egui::Event::Key {
                            key: Key::ArrowDown,
                            pressed: true,
                            repeat: false,
                            ..
                        } => cycle += 1,
                        _ => {}
                    }
                }
            }
            let allow = !input.modifiers.alt && !input.modifiers.mac_cmd;
            (
                allow && input.key_down(Key::ArrowLeft),
                allow && input.key_down(Key::ArrowRight),
                input.modifiers.ctrl,
                cycle,
            )
        });
        if !left && !right && cycle == 0 && self.owner != Some(viewport) {
            return;
        }
        if self.owner != Some(viewport) {
            self.cancel(character);
        }
        self.owner = Some(viewport);
        // A mouse/API action selected while a key is held becomes the action
        // restored on release, rather than being overwritten by stale state.
        if self
            .walking_animation
            .as_deref()
            .is_some_and(|name| name != character.animation().animation)
        {
            self.release(character);
        }
        if cycle != 0 {
            let choices: Vec<_> = animation::manual_animations(character.available_animations())
                .into_iter()
                .filter(|name| animation::animation_allowed(name, character.config()))
                .map(str::to_owned)
                .collect();
            if !choices.is_empty() {
                let current = self
                    .resting
                    .as_deref()
                    .unwrap_or(&character.animation().animation);
                let index = choices
                    .iter()
                    .position(|name| name == current)
                    .map(|index| index as i32)
                    .unwrap_or(if cycle > 0 { -1 } else { 0 });
                let next =
                    choices[(index + cycle).rem_euclid(choices.len() as i32) as usize].clone();
                if self.walking_animation.is_some() {
                    self.resting = Some(next);
                } else {
                    character.play_animation(next);
                }
            }
        }
        let direction = if left {
            Some(Direction::Left)
        } else if right {
            Some(Direction::Right)
        } else {
            None
        };
        if let Some(direction) = direction {
            character.set_direction(direction);
            if !character.config().allow_walk {
                self.release(character);
                return;
            }
            let movement =
                animation::movement_animation(character.available_animations()).map(str::to_owned);
            if let Some(movement) = movement {
                if self.walking_animation.is_none() {
                    self.resting = Some(character.animation().animation.clone());
                }
                if character.animation().animation != movement {
                    character.play_animation(movement.clone());
                }
                self.walking_animation = Some(movement);
                self.walk = Some(WalkControl { direction, fast });
                return;
            }
        }
        self.release(character);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character() -> CharacterState {
        CharacterState::new(
            arkpets::CharacterModel {
                animations: [
                    "Relax",
                    "Move",
                    "Sit",
                    "Attack_Loop",
                    "Skill_1_Loop",
                    "Skill_1_Begin",
                    "Start",
                ]
                .map(str::to_owned)
                .to_vec(),
                ..Default::default()
            },
            arkpets::PetConfig::default(),
            arkpets::Vec2::new(100.0, 100.0),
            1,
        )
    }

    fn key(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        let mut input = egui::RawInput {
            focused: true,
            events,
            ..Default::default()
        };
        input.viewports.get_mut(&ViewportId::ROOT).unwrap().focused = Some(true);
        input
    }

    fn frame(
        ctx: &Context,
        control: &mut Control,
        character: &mut CharacterState,
        input: egui::RawInput,
        enabled: bool,
    ) {
        ctx.run_ui(input, |_| control.update(ctx, character, enabled, true))
            .drop_without_applying_deltas();
    }

    #[test]
    fn an_unpaired_model_without_move_does_not_slide() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = CharacterState::new(
            arkpets::CharacterModel {
                animations: ["Idle", "Skill_1_Loop"].map(str::to_owned).to_vec(),
                ..Default::default()
            },
            arkpets::PetConfig::default(),
            arkpets::Vec2::new(100.0, 100.0),
            1,
        );
        pet.play_animation("Skill_1_Loop");
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, true)]),
            true,
        );
        assert!(control.walking().is_none());
        assert_eq!(pet.animation().animation, "Skill_1_Loop");
    }

    #[test]
    fn moving_takes_priority_and_action_changes_wait_for_release() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = character();
        pet.play_animation("Attack_Loop");
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, true)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Move");
        assert!(control.walking().is_some());
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowDown, true)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Move");
        assert!(control.walking().is_some());
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, false)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Skill_1_Loop");
        assert!(control.walking().is_none());
    }

    #[test]
    fn held_keys_accelerate_switch_direction_and_restore_the_selected_action() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = character();
        pet.play_animation("Sit");
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowLeft, true)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Move");
        let mut accelerated = input(vec![key(Key::ArrowRight, true)]);
        accelerated
            .events
            .push(egui::Event::ModifiersChanged(egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }));
        frame(&ctx, &mut control, &mut pet, accelerated, true);
        assert_eq!(
            control.walking(),
            Some(WalkControl {
                direction: Direction::Left,
                fast: true
            })
        );
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowLeft, false)]),
            true,
        );
        assert_eq!(control.walking().unwrap().direction, Direction::Right);
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, false)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Sit");
        assert!(control.walking().is_none());
    }

    #[test]
    fn action_cycling_preserves_raw_clips_and_skips_movement_and_disabled_actions() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = character();
        let mut config = pet.config().clone();
        config.allow_sit = false;
        pet.configure(config);
        for expected in [
            "Attack_Loop",
            "Skill_1_Loop",
            "Skill_1_Begin",
            "Start",
            "Relax",
        ] {
            frame(
                &ctx,
                &mut control,
                &mut pet,
                input(vec![key(Key::ArrowDown, true)]),
                true,
            );
            assert_eq!(pet.animation().animation, expected);
            frame(
                &ctx,
                &mut control,
                &mut pet,
                input(vec![key(Key::ArrowDown, true)]),
                true,
            );
            assert_eq!(
                pet.animation().animation,
                expected,
                "OS key repeat must not cycle again"
            );
            frame(
                &ctx,
                &mut control,
                &mut pet,
                input(vec![key(Key::ArrowDown, false)]),
                true,
            );
        }
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowUp, true)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Start");
    }

    #[test]
    fn focus_loss_widgets_and_mode_changes_cancel_movement_without_overwriting_mouse_actions() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = character();
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, true)]),
            true,
        );
        let mut unfocused = input(vec![]);
        unfocused.focused = false;
        unfocused
            .viewports
            .get_mut(&ViewportId::ROOT)
            .unwrap()
            .focused = Some(false);
        frame(&ctx, &mut control, &mut pet, unfocused, true);
        assert!(control.walking().is_none());
        assert_eq!(pet.animation().animation, "Relax");
        ctx.memory_mut(|memory| memory.request_focus(Id::new("search-text")));
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, true)]),
            true,
        );
        assert!(control.walking().is_none());
        ctx.memory_mut(|memory| memory.surrender_focus(Id::new("search-text")));
        frame(&ctx, &mut control, &mut pet, input(vec![]), true);
        assert!(control.walking().is_some());
        pet.play_animation("Attack_Loop");
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, false)]),
            true,
        );
        assert_eq!(pet.animation().animation, "Attack_Loop");
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowLeft, true)]),
            true,
        );
        frame(&ctx, &mut control, &mut pet, input(vec![]), false);
        assert!(control.walking().is_none());
        assert_eq!(pet.animation().animation, "Attack_Loop");
    }

    #[test]
    fn an_unfocused_second_viewport_does_not_cancel_the_controlling_viewport() {
        let ctx = Context::default();
        let mut control = Control::default();
        let mut pet = character();
        frame(
            &ctx,
            &mut control,
            &mut pet,
            input(vec![key(Key::ArrowRight, true)]),
            true,
        );
        let other = ViewportId::from_hash_of("other");
        let mut other_input = input(vec![]);
        other_input.viewport_id = other;
        other_input.focused = false;
        other_input.viewports.insert(
            other,
            egui::ViewportInfo {
                focused: Some(false),
                ..Default::default()
            },
        );
        frame(&ctx, &mut control, &mut pet, other_input, true);
        assert!(control.walking().is_some());
        assert_eq!(control.owner(), Some(ViewportId::ROOT));
    }
}

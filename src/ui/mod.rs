mod backend;
mod desktop;
mod effects;
mod forms;
mod keyboard;
#[cfg(target_os = "macos")]
mod menu_bar;
mod opacity;
mod performance;
#[cfg(windows)]
mod platform;
mod renderer;
mod shell_visibility;
#[cfg(debug_assertions)]
mod smoke;
mod transition;

use arkpets::{
    config::{EasingFunction, OutlineCondition},
    interaction::DirectionGesture,
    runtime::CommandKind,
    Bounds, CharacterModel, CharacterState, Direction, DirectionSwitching, ModelCatalog,
    PetCommand, PetConfig, PetRuntime, TransitionPreset, Vec2 as PetVec2,
};
use backend::RendererBackend;
use eframe::egui::{
    self, Align, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_phosphor::regular as icons;
use renderer::Renderer;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(0, 126, 111);
const MUTED: Color32 = Color32::from_rgb(109, 118, 123);
const INK: Color32 = Color32::from_rgb(36, 44, 47);

fn desktop_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("desktop-pet")
}

struct NativeApp {
    state: Arc<Mutex<App>>,
    #[cfg(target_os = "macos")]
    menu_bar: menu_bar::MenuBar,
    #[cfg(target_os = "macos")]
    dock_visible: Option<bool>,
}

#[cfg(target_os = "macos")]
impl NativeApp {
    fn sync_dock_visibility(&mut self) {
        let show = self.state.lock().unwrap().prefs.show_in_taskbar;
        if self.dock_visible != Some(show) {
            match shell_visibility::set_application_visible(show) {
                Ok(()) => self.dock_visible = Some(show),
                Err(error) => self.state.lock().unwrap().error = Some(error),
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    config: PetConfig,
    selected: String,
    automatic: bool,
    dark: bool,
    fps: u32,
    always_on_top: bool,
    click_through: bool,
    show_in_taskbar: bool,
    auto_taskbar_height: bool,
    taskbar_height: u32,
    multi_monitors: bool,
    initial_position: [f32; 2],
    form_preferences: Vec<forms::Preference>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            config: PetConfig::default(),
            selected: "pepe".into(),
            automatic: true,
            dark: false,
            fps: 60,
            always_on_top: true,
            click_through: false,
            show_in_taskbar: false,
            auto_taskbar_height: cfg!(windows),
            taskbar_height: 48,
            multi_monitors: true,
            initial_position: [0.2, 0.2],
            form_preferences: Vec::new(),
        }
    }
}

impl Preferences {
    fn bottom_height(&self) -> Option<u32> {
        (!self.auto_taskbar_height || !cfg!(windows)).then_some(self.taskbar_height)
    }
}

pub struct App {
    catalog: ModelCatalog,
    asset_root: PathBuf,
    prefs: Preferences,
    runtime: Option<PetRuntime>,
    renderer: Option<Renderer>,
    forms: Option<forms::Forms>,
    compositor: renderer::Compositor,
    selected: usize,
    search: String,
    favorites: Vec<String>,
    favorites_only: bool,
    error: Option<String>,
    page: usize,
    import_open: bool,
    import_path: String,
    desktop_open: bool,
    desktop_motion: Option<desktop::DesktopMotion>,
    desktop_opacity: Option<opacity::OpacityFade>,
    desktop_presentation: transition::Presentation,
    preview_presentation: transition::Presentation,
    preview_properties_animating: bool,
    preview_effects: effects::EffectState,
    desktop_effects: effects::EffectState,
    last_frame: Instant,
    next_frame: Instant,
    preview_next_frame: Instant,
    preview_direction: Direction,
    preview_gesture: DirectionGesture,
    keyboard: keyboard::Control,
    animation_name: String,
    first_position: bool,
    frame_ms: f32,
    gpu: String,
    theme_applied: bool,
    profile: Option<performance::Profile>,
    backend: RendererBackend,
    pending_backend: Option<RendererBackend>,
    switch_viewport: Option<egui::ViewportBuilder>,
    #[cfg(debug_assertions)]
    capture: Option<smoke::Capture>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, backend: RendererBackend) -> Self {
        eprintln!("Initializing ArkPets UI");
        let profile = performance::Profile::from_env().expect("performance configuration");
        let prefs = if let Some(profile) = &profile {
            Preferences {
                selected: profile.settings().model.clone(),
                fps: profile.settings().fps,
                automatic: false,
                ..Default::default()
            }
        } else {
            cc.storage
                .and_then(|s| eframe::get_value(s, "preferences"))
                .unwrap_or_default()
        };
        let root = asset_root();
        configure_fonts(&cc.egui_ctx);
        let (catalog, error) = match ModelCatalog::from_json_file(root.join("models.json")) {
            Ok(catalog) => (catalog, None),
            Err(error) => (ModelCatalog::default(), Some(error.to_string())),
        };
        let gpu = renderer_name(cc);
        eprintln!("Renderer: {gpu}");
        let mut result = Self {
            catalog,
            asset_root: root,
            prefs,
            runtime: None,
            renderer: None,
            forms: None,
            compositor: renderer::Compositor::new(cc),
            selected: 0,
            search: String::new(),
            favorites: cc
                .storage
                .and_then(|s| eframe::get_value(s, "favorites"))
                .unwrap_or_default(),
            favorites_only: false,
            error,
            page: 0,
            import_open: false,
            import_path: String::new(),
            desktop_open: false,
            desktop_motion: None,
            desktop_opacity: None,
            desktop_presentation: transition::Presentation::default(),
            preview_presentation: transition::Presentation::default(),
            preview_properties_animating: false,
            preview_effects: effects::EffectState::default(),
            desktop_effects: effects::EffectState::default(),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            preview_next_frame: Instant::now(),
            preview_direction: Direction::Right,
            preview_gesture: DirectionGesture::default(),
            keyboard: keyboard::Control::default(),
            animation_name: String::new(),
            first_position: true,
            frame_ms: 0.0,
            gpu,
            theme_applied: false,
            profile,
            backend,
            pending_backend: None,
            switch_viewport: None,
            #[cfg(debug_assertions)]
            capture: smoke::Capture::from_env(),
        };
        result.theme(&cc.egui_ctx);
        if let (Some(profile), Some(state)) = (&result.profile, &cc.wgpu_render_state) {
            profile.record_device(state.device.clone());
        }
        let index = result
            .catalog
            .models
            .iter()
            .position(|model| model.id == result.prefs.selected)
            .unwrap_or(0);
        if !result.catalog.models.is_empty() {
            result.select(index, &cc.egui_ctx);
        }
        if let Some(profile) = &result.profile {
            assert_eq!(
                result.prefs.selected,
                profile.settings().model,
                "profile model must exist"
            );
            result.desktop_open = true;
            result.page = usize::from(profile.settings().page == "settings");
        }
        if let Some(profile) = &result.profile {
            if let Some(form) = result.forms.as_ref().and_then(|forms| forms.active_form()) {
                result
                    .select_form(profile.settings().form_mode.resolve(form))
                    .expect("configure diagnostic form");
            }
        } else if let Some(form) = result.forms.as_ref().and_then(|forms| forms.active_form()) {
            // Persist an old selected model ID as an explicit form preference.
            if let Err(error) = result.select_form(form) {
                result.error = Some(error);
            }
        }
        result
    }

    fn rebind_renderer(
        &mut self,
        cc: &eframe::CreationContext<'_>,
        backend: RendererBackend,
    ) -> Result<(), String> {
        self.cancel_keyboard();
        configure_fonts(&cc.egui_ctx);
        self.compositor = renderer::Compositor::new(cc);
        self.theme(&cc.egui_ctx);
        self.theme_applied = false;
        self.backend = backend;
        self.gpu = renderer_name(cc);
        self.last_frame = Instant::now();
        self.next_frame = self.last_frame;
        self.preview_next_frame = self.last_frame;
        if let Some(motion) = &mut self.desktop_motion {
            // The recreated pet HWND must be positioned and shown on its first frame.
            motion.last_window_position = None;
            motion.taskbar_visibility = None;
            motion.last_tick = self.last_frame;
            #[cfg(windows)]
            {
                motion.appearance_ready = false;
            }
        }
        if let (Some(profile), Some(state)) = (&self.profile, &cc.wgpu_render_state) {
            profile.record_device(state.device.clone());
        }
        eprintln!("Renderer: {}", self.gpu);
        Ok(())
    }

    fn theme(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.prefs.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let mut visuals = if self.prefs.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.window_corner_radius = egui::CornerRadius::same(8);
        if self.prefs.dark {
            visuals.override_text_color = Some(Color32::from_rgb(227, 233, 230));
        }
        if !self.prefs.dark {
            visuals.panel_fill = Color32::WHITE;
            visuals.override_text_color = Some(INK);
            visuals.faint_bg_color = Color32::from_rgb(242, 246, 245);
            visuals.extreme_bg_color = Color32::from_rgb(248, 250, 249);
        }
        ctx.set_visuals(visuals);
        ctx.style_mut_of(
            if self.prefs.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            },
            |s| {
                s.spacing.item_spacing = Vec2::new(10.0, 10.0);
                s.spacing.button_padding = Vec2::new(12.0, 8.0);
                s.text_styles
                    .insert(egui::TextStyle::Body, FontId::proportional(14.0));
                s.text_styles
                    .insert(egui::TextStyle::Button, FontId::proportional(14.0));
            },
        );
    }

    fn select(&mut self, index: usize, ctx: &egui::Context) {
        match forms::Forms::load(
            &self.catalog,
            index,
            &self.asset_root,
            self.prefs.config.rendering.canvas,
            &self.prefs.form_preferences,
        ) {
            Ok((forms, renderer)) => {
                let active = forms.active;
                self.install_model(active, self.catalog.models[active].clone(), renderer);
                self.forms = Some(forms);
                self.sync_animation(false);
            }
            Err(error) => self.error = Some(error),
        }
        ctx.request_repaint();
    }

    fn install_model(&mut self, index: usize, mut model: CharacterModel, renderer: Renderer) {
        self.forms = None;
        self.keyboard = keyboard::Control::default();
        model.animations = renderer.spine.animations.clone();
        let character = CharacterState::new(
            model.clone(),
            self.prefs.config.clone(),
            PetVec2::new(160.0, 220.0),
            42 + index as u64,
        );
        self.animation_name = character.animation().animation.clone();
        self.prefs.selected = model.id;
        self.selected = index;
        if let Some(runtime) = &mut self.runtime {
            *runtime.character_mut() = character;
        } else {
            let mut runtime = PetRuntime::new(character);
            runtime
                .apply(PetCommand::SetAutomatic(self.prefs.automatic))
                .unwrap();
            self.runtime = Some(runtime);
        }
        self.renderer = Some(renderer);
        self.preview_presentation = transition::Presentation::default();
        self.desktop_presentation = transition::Presentation::default();
        self.preview_effects = effects::EffectState::default();
        self.desktop_effects = effects::EffectState::default();
        self.first_position = true;
        self.error = None;
    }

    fn sync_animation(&mut self, force: bool) {
        let dragging = self
            .desktop_motion
            .as_ref()
            .is_some_and(|motion| motion.body.dragging);
        if let Some(runtime) = &mut self.runtime {
            let redirected = !dragging
                && !runtime.paused()
                && !runtime.automatic()
                && self.keyboard.prioritize_walk(runtime.character_mut());
            let name = runtime.character().animation().animation.clone();
            if let Err(error) = self.play_animation(&name, force && !redirected) {
                self.error = Some(format!("播放动作失败: {error}"));
                self.runtime
                    .as_mut()
                    .unwrap()
                    .apply(PetCommand::SetPaused(true))
                    .unwrap();
            }
        }
    }

    fn play_animation(&mut self, name: &str, force: bool) -> Result<(), String> {
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        if force || self.animation_name != name {
            renderer
                .spine
                .play_with_mix(
                    name,
                    arkpets::animation::animation_loops(name),
                    self.prefs.config.animation_mix_duration,
                )
                .map_err(|error| error.to_string())?;
            renderer.update(0.0, self.prefs.config.animation_mix_duration)?;
            self.animation_name = name.to_owned();
        }
        Ok(())
    }

    fn select_form(&mut self, form: arkpets::model::ModelForm) -> Result<(), String> {
        let forms = self.forms.as_ref().ok_or("当前模型未配置形态")?;
        let target = forms.index_for(form)?;
        if target != forms.active {
            self.cancel_keyboard();
            let forms = self.forms.as_mut().unwrap();
            let renderer = self.renderer.as_mut().unwrap();
            let next = forms.prepare(target, renderer.canvas_settings)?;
            if let Some(motion) = &mut self.desktop_motion {
                let previous_display = motion.last_window_position;
                let layout = next.desktop_layout(
                    180.0 * self.prefs.config.display_scale,
                    effects::padding(&self.prefs.config),
                    motion.pixels_per_point,
                );
                let delta = motion
                    .reframe(layout.body.size())
                    .map_err(|error| error.to_string())?;
                self.desktop_presentation.translate_position(delta);
                if let (Some(previous), Some(shown)) =
                    (previous_display, motion.last_window_position)
                {
                    if (previous + delta).distance(shown) > 0.001 {
                        self.desktop_presentation
                            .reset_position(shown, Instant::now());
                    }
                }
            }
            forms.activate(target, renderer);
            let mut model = self.catalog.models[target].clone();
            model.animations = renderer.spine.animations.clone();
            let character = self.runtime.as_mut().unwrap().character_mut();
            character.set_model_form(model);
            self.animation_name = character.animation().animation.clone();
            self.selected = target;
            self.prefs.selected = self.catalog.models[target].id.clone();
        }
        let model = &self.catalog.models[target];
        if let (Some(character_id), Some(skin_id)) = (&model.character_id, &model.skin_id) {
            self.prefs
                .form_preferences
                .retain(|value| !value.matches(model));
            self.prefs.form_preferences.push(forms::Preference {
                character_id: character_id.clone(),
                skin_id: skin_id.clone(),
                mode: match form {
                    arkpets::model::ModelForm::Base => forms::Mode::Base,
                    arkpets::model::ModelForm::Battle => forms::Mode::Battle,
                },
                preferred: form,
            });
        }
        Ok(())
    }

    fn paused(&self) -> bool {
        self.runtime.as_ref().is_some_and(PetRuntime::paused)
    }

    fn set_paused(&mut self, paused: bool) {
        if paused {
            self.cancel_keyboard();
        }
        if let Some(runtime) = &mut self.runtime {
            runtime.apply(PetCommand::SetPaused(paused)).unwrap();
        }
    }

    fn cancel_keyboard(&mut self) {
        if let Some(runtime) = &mut self.runtime {
            self.keyboard.cancel(runtime.character_mut());
            self.sync_animation(false);
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let viewport = ctx.viewport_id();
        let viewport_enabled = if viewport == desktop_id() {
            self.desktop_open && !self.prefs.click_through
        } else {
            self.page == 0 && !self.import_open
        };
        let dragging = self
            .desktop_motion
            .as_ref()
            .is_some_and(|motion| motion.body.dragging);
        if let Some(runtime) = &mut self.runtime {
            let enabled = !runtime.automatic() && !runtime.paused() && !dragging;
            let before = runtime.character().animation().clone();
            self.keyboard
                .update(ctx, runtime.character_mut(), enabled, viewport_enabled);
            if *runtime.character().animation() != before {
                self.sync_animation(false);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
                if self.desktop_open {
                    ctx.request_repaint_of(desktop_id());
                }
            }
        }
        if let (Some(profile), Some(runtime)) = (&self.profile, &self.runtime) {
            let active = self
                .forms
                .as_ref()
                .map_or(self.selected, |forms| forms.active);
            profile
                .record_controls(
                    runtime,
                    self.keyboard.walking(),
                    &self.catalog.models[active].id,
                    self.forms.as_ref().and_then(|forms| forms.active_form()),
                )
                .expect("write control diagnostic");
        }
    }

    fn advance(&mut self, ctx: &egui::Context, dt: f32) {
        let mut force = false;
        if let Some(runtime) = &mut self.runtime {
            let processed = runtime.drain_commands(|event| {
                if let Err(error) = event.result {
                    self.error = Some(error.to_string());
                } else if matches!(
                    event.command,
                    CommandKind::PlayAnimation | CommandKind::Interact
                ) {
                    force = true;
                }
            });
            if processed == arkpets::runtime::COMMANDS_PER_FRAME {
                ctx.request_repaint();
            }
            self.prefs.automatic = runtime.automatic();
            self.prefs.config = runtime.character().config().clone();
            if processed > 0 {
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        }
        self.sync_animation(force);
        if !self.paused() && dt > 0.0 {
            if let Some(renderer) = &mut self.renderer {
                match renderer.update(dt, self.prefs.config.animation_mix_duration) {
                    Ok(true) => {
                        if let Some(runtime) = &mut self.runtime {
                            if !runtime.character().physics().dragging
                                && !self
                                    .desktop_motion
                                    .as_ref()
                                    .is_some_and(|m| m.body.dragging)
                            {
                                runtime.animation_completed();
                            }
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.set_paused(true);
                        ctx.request_repaint_of(egui::ViewportId::ROOT);
                    }
                    _ => {}
                }
            }
        }
        self.sync_animation(false);
    }

    fn sidebar(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        egui::Panel::left("library")
            .exact_size(226.0)
            .resizable(false)
            .show(root_ui, |ui| {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icons::CUBE).size(30.0).color(ACCENT));
                    ui.label(RichText::new("ArkPets").size(27.0).strong());
                });
                ui.add_space(24.0);
                for (index, icon, text) in [
                    (0, icons::SQUARES_FOUR, "模型库"),
                    (1, icons::SLIDERS_HORIZONTAL, "行为与显示"),
                ] {
                    let selected = self.page == index;
                    if ui
                        .add_sized(
                            [202.0, 38.0],
                            egui::Button::new(RichText::new(format!("{icon}    {text}")).color(
                                if selected {
                                    Color32::WHITE
                                } else if self.prefs.dark {
                                    Color32::from_gray(225)
                                } else {
                                    INK
                                },
                            ))
                            .selected(selected),
                        )
                        .clicked()
                    {
                        self.page = index;
                    }
                }
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("本地模型").small().color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, icons::PLUS, "导入模型或清单").clicked() {
                            self.import_open = true;
                        }
                    });
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索名称 / ID")
                        .desired_width(202.0),
                );
                ui.checkbox(&mut self.favorites_only, "仅收藏");
                let searched = self.catalog.search(&self.search);
                let matching: Vec<usize> = self
                    .catalog
                    .character_indices()
                    .into_iter()
                    .filter(|&index| {
                        searched
                            .iter()
                            .any(|model| model.same_character(&self.catalog.models[index]))
                    })
                    .filter(|&index| {
                        !self.favorites_only
                            || self.catalog.models.iter().any(|model| {
                                model.same_character(&self.catalog.models[index])
                                    && self.favorites.contains(&model.id)
                            })
                    })
                    .collect();
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 100.0).max(120.0))
                    .show(ui, |ui| {
                        if matching.is_empty() {
                            ui.label(RichText::new("没有匹配的模型").color(MUTED));
                        }
                        for index in matching {
                            let model = &self.catalog.models[index];
                            let selected = model
                                .same_character(&self.catalog.models[self.selected])
                                && self.renderer.is_some();
                            let title = format!("{}\n{}", model.name, model.id);
                            let button = egui::Button::new(RichText::new(title).size(14.0).color(
                                if selected {
                                    Color32::WHITE
                                } else if self.prefs.dark {
                                    Color32::from_gray(225)
                                } else {
                                    INK
                                },
                            ))
                            .selected(selected);
                            if ui.add_sized([202.0, 60.0], button).clicked() {
                                self.select(index, &ctx);
                                self.page = 0;
                            }
                        }
                    });
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!(
                            "{} 个角色 · {} 套模型",
                            self.catalog.character_indices().len(),
                            self.catalog.models.len()
                        ))
                        .small()
                        .color(MUTED),
                    );
                    ui.label(RichText::new("ARKPETS.RS  /  0.1.0").small().color(MUTED));
                    ui.separator();
                });
            });
    }

    fn toolbar(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        egui::Panel::top("toolbar")
            .exact_size(72.0)
            .show(root_ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                });
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(if self.page == 0 {
                            "模型库"
                        } else {
                            "行为与显示"
                        })
                        .size(20.0)
                        .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let launch = egui::Button::new(
                            RichText::new(format!(
                                "{}  {}",
                                icons::ARROW_SQUARE_OUT,
                                if self.desktop_open {
                                    "收起桌宠"
                                } else {
                                    "启动桌宠"
                                }
                            ))
                            .color(Color32::WHITE),
                        )
                        .fill(ACCENT);
                        if ui.add_enabled(self.renderer.is_some(), launch).clicked() {
                            self.desktop_open = !self.desktop_open;
                        }
                        if icon_button(
                            ui,
                            if self.prefs.dark {
                                icons::SUN
                            } else {
                                icons::MOON
                            },
                            "切换外观",
                        )
                        .clicked()
                        {
                            self.prefs.dark = !self.prefs.dark;
                            self.theme(&ctx);
                        }
                    });
                });
            });
    }

    fn inspector(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        if self.page != 0 {
            return;
        }
        let mut paused = self.paused();
        let original_forms = self.forms.as_ref().and_then(|forms| forms.active_form());
        let mut form_options = original_forms;
        egui::Panel::right("inspector")
            .exact_size(250.0)
            .resizable(false)
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(20.0);
                    let Some(character) = self.runtime.as_mut().map(PetRuntime::character_mut)
                    else {
                        ui.label("未载入模型");
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("角色").small().color(MUTED));
                        let representative =
                            &self.catalog.models[self.catalog.representative(self.selected)];
                        let id = representative.id.clone();
                        let family_ids: Vec<_> = self
                            .catalog
                            .models
                            .iter()
                            .filter(|model| model.same_character(representative))
                            .map(|model| model.id.clone())
                            .collect();
                        let favorite = self.favorites.iter().any(|id| family_ids.contains(id));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(
                                ui,
                                if favorite {
                                    icons::STAR_HALF
                                } else {
                                    icons::STAR
                                },
                                "收藏角色",
                            )
                            .clicked()
                            {
                                if favorite {
                                    self.favorites.retain(|value| !family_ids.contains(value));
                                } else {
                                    self.favorites.push(id);
                                }
                            }
                        });
                    });
                    ui.label(
                        RichText::new(
                            &self.catalog.models[self.catalog.representative(self.selected)].name,
                        )
                        .size(21.0)
                        .strong(),
                    );
                    ui.label(RichText::new(&character.model().id).small().color(MUTED));
                    if let (Some(forms), Some(selected)) = (&self.forms, &mut form_options) {
                        use arkpets::model::ModelForm;
                        if forms.has(ModelForm::Base) && forms.has(ModelForm::Battle) {
                            egui::ComboBox::from_label("形态")
                                .selected_text(forms::label(*selected))
                                .show_ui(ui, |ui| {
                                    for form in [ModelForm::Base, ModelForm::Battle] {
                                        ui.selectable_value(selected, form, forms::label(form));
                                    }
                                });
                        }
                    }
                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(RichText::new("动作").strong());
                    let mut force = false;
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.prefs.automatic, true, "自动");
                        let was_automatic = self.prefs.automatic;
                        if ui
                            .selectable_value(&mut self.prefs.automatic, false, "手动")
                            .clicked()
                        {
                            if was_automatic {
                                force |= character.play_idle();
                            }
                            ctx.memory_mut(|memory| {
                                memory.request_focus(keyboard::target_id(egui::ViewportId::ROOT))
                            });
                        }
                    });
                    if !self.prefs.automatic {
                        ui.small("←/→ 行走 · Ctrl 加速 · ↑/↓ 切换动作");
                        ui.small("点击预览或桌宠获取控制焦点；松开恢复所选动作。");
                        if arkpets::animation::movement_animation(character.available_animations())
                            .is_none()
                        {
                            ui.small("没有可用 Move 动画，无法按键行走。");
                        }
                        if let Some(resting) = self.keyboard.resting_action() {
                            ui.small(format!("松开方向键后：{resting}"));
                        }
                    }
                    let mut selected = character.animation().animation.clone();
                    egui::ComboBox::from_id_salt("animation")
                        .width(220.0)
                        .selected_text(&selected)
                        .show_ui(ui, |ui| {
                            for animation in character.available_animations() {
                                ui.selectable_value(&mut selected, animation.clone(), animation);
                            }
                        });
                    if selected != character.animation().animation {
                        if character.play_animation(selected) {
                            self.prefs.automatic = false;
                            force = true;
                        } else {
                            self.error = Some("该动作已在行为设置中关闭".into());
                        }
                    }
                    ui.horizontal(|ui| {
                        if icon_button(ui, icons::ARROW_LEFT, "朝左").clicked() {
                            character.set_direction(Direction::Left);
                        }
                        if icon_button(ui, icons::HAND_TAP, "互动").clicked() {
                            force |= character.click();
                        }
                        if icon_button(ui, icons::ARROW_RIGHT, "朝右").clicked() {
                            character.set_direction(Direction::Right);
                        }
                        if icon_button(
                            ui,
                            if paused { icons::PLAY } else { icons::PAUSE },
                            "播放 / 暂停",
                        )
                        .clicked()
                        {
                            paused = !paused;
                        }
                    });
                    ui.add_space(16.0);
                    ui.separator();
                    ui.label(RichText::new("显示").strong());
                    ui.add(
                        egui::Slider::new(&mut self.prefs.config.display_scale, 0.5..=1.7)
                            .text("缩放")
                            .suffix(" x"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.prefs.config.opacity_normal, 0.1..=1.0)
                            .text("不透明度"),
                    );
                    ui.checkbox(&mut self.prefs.always_on_top, "桌宠置顶");
                    if ui
                        .checkbox(&mut self.prefs.click_through, "透明模式（鼠标穿透）")
                        .on_hover_text(
                            "使用透明模式不透明度，鼠标可操作桌宠后方的窗口；可在主界面关闭。",
                        )
                        .changed()
                    {
                        ctx.request_repaint_of(desktop_id());
                    }
                    ui.add_space(16.0);
                    ui.separator();
                    if let Some(renderer) = &self.renderer {
                        ui.label(RichText::new("资源").strong());
                        metadata(ui, "Spine", &renderer.spine.version);
                        metadata(ui, "动画", &renderer.spine.animations.len().to_string());
                        metadata(ui, "纹理页", &renderer.spine.pages.len().to_string());
                        metadata(ui, "三角形", &renderer.triangles.to_string());
                    }
                    if force {
                        ctx.memory_mut(|memory| {
                            memory.request_focus(keyboard::target_id(egui::ViewportId::ROOT))
                        });
                        self.sync_animation(true);
                    }
                });
            });
        if form_options != original_forms {
            if let Some(form) = form_options {
                if let Err(error) = self.select_form(form) {
                    self.error = Some(error);
                } else {
                    self.error = None;
                }
                ctx.request_repaint();
                if self.desktop_open {
                    ctx.request_repaint_of(desktop_id());
                }
            }
        }
        if paused != self.paused() {
            if let Some(runtime) = &self.runtime {
                let ctx = ctx.clone();
                let sender = runtime.command_sender().with_waker(Arc::new(move || {
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                    ctx.request_repaint_of(desktop_id());
                }));
                if let Err(error) = sender.try_send(PetCommand::SetPaused(paused)) {
                    self.error = Some(error.to_string());
                }
            }
        }
    }

    fn preview(&mut self, root_ui: &mut egui::Ui, dt: f32) {
        let ctx = root_ui.ctx().clone();
        let paused = self.paused();
        self.preview_properties_animating = false;
        let now = Instant::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(root_ui.visuals().panel_fill))
            .show(root_ui, |ui| {
                if self.page == 1 {
                    self.settings(ui);
                    return;
                }
                let available = ui.available_size();
                let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
                let control_response = ui.interact(
                    rect,
                    keyboard::target_id(egui::ViewportId::ROOT),
                    Sense::click(),
                );
                keyboard::focus_response(&control_response);
                // Native windows can briefly report a tiny content area during
                // creation or minimization, before their minimum size is applied.
                if rect.width() <= 40.0 || rect.height() <= 100.0 {
                    return;
                }
                let painter = ui.painter_at(rect);
                let background = if self.prefs.dark {
                    Color32::from_rgb(30, 34, 34)
                } else {
                    Color32::from_rgb(243, 247, 246)
                };
                painter.rect_filled(rect, 0.0, background);
                let grid = if self.prefs.dark {
                    Color32::from_gray(47)
                } else {
                    Color32::from_rgb(222, 232, 229)
                };
                let mut x = rect.left() + 24.0;
                while x < rect.right() {
                    painter.line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        Stroke::new(0.5_f32, grid),
                    );
                    x += 32.0;
                }
                let mut y = rect.top() + 24.0;
                while y < rect.bottom() {
                    painter.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(0.5_f32, grid),
                    );
                    y += 32.0;
                }
                painter.text(
                    rect.left_top() + Vec2::new(24.0, 24.0),
                    egui::Align2::LEFT_TOP,
                    "预览",
                    FontId::proportional(13.0),
                    MUTED,
                );
                let (Some(runtime), Some(renderer)) = (&mut self.runtime, &self.renderer) else {
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "尚未载入模型",
                        FontId::proportional(18.0),
                        MUTED,
                    );
                    return;
                };
                let character = runtime.character_mut();
                let height = (240.0 * self.prefs.config.display_scale).min(rect.height() * 0.7);
                let size = renderer
                    .desktop_layout(height, 1.0, ctx.pixels_per_point())
                    .body
                    .size();
                let fit = ((rect.width() - 40.0) / size.x)
                    .min((rect.height() - 100.0) / size.y)
                    .min(1.0);
                let layout = renderer.desktop_layout(height * fit, 1.0, ctx.pixels_per_point());
                let size = layout.body.size();
                let body_size = PetVec2::new(size.x, size.y);
                if self.first_position {
                    character.resize(body_size);
                } else {
                    let delta = character.resize_around_feet(body_size);
                    self.preview_presentation
                        .translate_position(Vec2::new(delta.x, delta.y));
                }
                let bounds = Bounds::new(rect.width(), (rect.height() - 70.0).max(size.y));
                character.set_position(character.position(), bounds);
                if self.first_position {
                    character.set_position(
                        PetVec2::new((rect.width() - size.x) * 0.5, bounds.height - size.y),
                        bounds,
                    );
                    self.first_position = false;
                }
                let desktop_direction = character.animation().direction;
                if self.desktop_open {
                    character.set_direction(self.preview_direction);
                }
                if !paused {
                    character.tick_controlled(dt, bounds, self.keyboard.walking());
                }
                self.preview_direction = character.animation().direction;
                let p = character.position();
                let (visual_position, _) = self.preview_presentation.position(
                    Pos2::new(p.x, p.y),
                    character.physics().dragging || self.keyboard.walking().is_some(),
                    &self.prefs.config,
                    now,
                );
                let paint_at = |position: Pos2| {
                    Rect::from_min_size(
                        rect.min + position.to_vec2() + (layout.paint.min - layout.body.min),
                        layout.paint.size(),
                    )
                };
                let mut target = paint_at(visual_position);
                let response = ui.interact(target, ui.id().with("pet"), Sense::click_and_drag());
                keyboard::focus_response(&response);
                let mut force = false;
                if response.clicked() {
                    force |= character.click();
                }
                if response.drag_started() && self.prefs.config.allow_drag {
                    if let Some(point) = ctx.input(|input| input.pointer.press_origin()) {
                        character.set_position(
                            PetVec2::new(visual_position.x, visual_position.y),
                            bounds,
                        );
                        character
                            .start_drag(PetVec2::new(point.x - rect.left(), point.y - rect.top()));
                        self.preview_gesture
                            .start_drag(point.x * ctx.pixels_per_point());
                        force |= self.prefs.config.allow_interact;
                    }
                }
                if response.dragged() {
                    if let Some(point) = response.interact_pointer_pos() {
                        character.drag_to(
                            PetVec2::new(point.x - rect.left(), point.y - rect.top()),
                            dt,
                            bounds,
                        );
                    }
                }
                if let Some(direction) = self.preview_gesture.update(
                    ctx.input(|input| input.pointer.latest_pos())
                        .map(|p| p.x * ctx.pixels_per_point()),
                    response.hovered(),
                    &self.prefs.config,
                ) {
                    character.set_direction(direction);
                }
                if response.drag_stopped() {
                    character.end_drag();
                    if let Some(direction) = self.preview_gesture.release(&self.prefs.config) {
                        character.set_direction(direction);
                    }
                }
                if self.keyboard.prioritize_walk(character) {
                    force = false;
                }
                self.preview_direction = character.animation().direction;
                let p = character.position();
                let (visual_position, moving) = self.preview_presentation.position(
                    Pos2::new(p.x, p.y),
                    character.physics().dragging || self.keyboard.walking().is_some(),
                    &self.prefs.config,
                    now,
                );
                target = paint_at(visual_position);
                let (facing, turning) = self.preview_presentation.facing(
                    character.animation().direction,
                    &self.prefs.config,
                    now,
                );
                self.preview_properties_animating = moving || turning;
                let (effect, outline_animating) = self.preview_effects.update(
                    &self.prefs.config,
                    effects::Input {
                        dragging: character.physics().dragging,
                        pressing: response.is_pointer_button_down_on(),
                        focused: ctx.input(|input| input.viewport().focused.unwrap_or(false)),
                        controlled: !self.prefs.automatic,
                    },
                    self.prefs.config.opacity_normal,
                    ctx.pixels_per_point(),
                    now,
                );
                self.preview_properties_animating |= outline_animating;
                if !character.is_hidden() {
                    renderer.paint_with_facing(&painter, target, facing, effect, &self.compositor);
                }
                if self.desktop_open {
                    character.set_direction(desktop_direction);
                }
                painter.text(
                    rect.center_bottom() - Vec2::new(0.0, 28.0),
                    egui::Align2::CENTER_BOTTOM,
                    &self.catalog.models[self.catalog.representative(self.selected)].name,
                    FontId::proportional(18.0),
                    if self.prefs.dark { Color32::WHITE } else { INK },
                );
                if force {
                    self.sync_animation(true);
                }
            });
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::NONE.inner_margin(28.0).show(ui, |ui| {
                ui.set_max_width(600.0);
                ui.heading("行为");
                ui.add_space(12.0);
                ui.checkbox(&mut self.prefs.config.allow_walk, "允许行走");
                ui.checkbox(&mut self.prefs.config.allow_sit, "允许坐下");
                ui.checkbox(&mut self.prefs.config.allow_sleep, "允许睡眠");
                ui.checkbox(&mut self.prefs.config.allow_special, "允许特殊动作");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.prefs.config.allow_interact, "允许触发交互动画")
                        .on_hover_text("关闭点击和自动选择的 Interact 动画；仍可手动播放，拖拽和翻转独立设置。");
                    ui.checkbox(&mut self.prefs.config.allow_drag, "允许拖拽");
                });
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.walk_speed, 0.0..=200.0)
                        .text("行走速度")
                        .suffix(" 逻辑像素/秒"),
                );
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.activity_level, 0..=16)
                        .text("动作活跃度")
                        .suffix(" 级"),
                )
                .on_hover_text("数值越高，自动动作越少选择待机；不会改变动画播放速度。");
                let direction_label = |mode| match mode {
                    DirectionSwitching::Disabled => "禁用",
                    DirectionSwitching::DragRelease => "松开拖拽时",
                    DirectionSwitching::Drag => "拖拽时",
                    DirectionSwitching::Hover => "光标掠过时",
                };
                egui::ComboBox::from_label("交互式朝向翻转")
                    .selected_text(direction_label(self.prefs.config.direction_switching))
                    .show_ui(ui, |ui| {
                        for mode in [
                            DirectionSwitching::Disabled,
                            DirectionSwitching::DragRelease,
                            DirectionSwitching::Drag,
                            DirectionSwitching::Hover,
                        ] {
                            ui.selectable_value(
                                &mut self.prefs.config.direction_switching,
                                mode,
                                direction_label(mode),
                            );
                        }
                    })
                    .response
                    .on_hover_text(
                        "跟随鼠标的水平移动方向；后面的选项同时包含前面的拖拽触发方式。",
                    );
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("物理");
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.gravity, 0.0..=2000.0)
                        .text("重力加速度").suffix(" 逻辑像素/秒²"),
                );
                let friction_changed = ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.air_friction, 0.0..=2000.0)
                        .text("空气阻力加速度").suffix(" 逻辑像素/秒²"),
                ).on_hover_text("在空中和地面持续减速，速度降到零后不会反向。").changed();
                let friction_changed = ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.ground_friction, 0.0..=2000.0)
                        .text("地面摩擦加速度").suffix(" 逻辑像素/秒²"),
                ).on_hover_text("接触当前屏幕底边时，额外减小水平惯性。").changed() || friction_changed;
                if friction_changed {
                    self.prefs.config.physics.drag = 1.0;
                    self.prefs.config.physics.min_velocity = 0.0;
                }
                ui.add(egui::Slider::new(&mut self.prefs.config.physics.max_velocity.x, 0.0..=2000.0)
                    .text("最大水平运动速率").suffix(" 逻辑像素/秒"))
                    .on_hover_text("限制惯性和拖拽释放速度；0 停止水平惯性，行走速度独立设置。");
                ui.add(egui::Slider::new(&mut self.prefs.config.physics.max_velocity.y, 0.0..=2000.0)
                    .text("最大垂直运动速率").suffix(" 逻辑像素/秒"))
                    .on_hover_text("限制下落和拖拽释放速度；0 停止垂直惯性。");
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.bounce_damping, 0.0..=1.0)
                        .text("反弹系数"),
                );
                if self.prefs.config.physics.drag != 1.0 || self.prefs.config.physics.min_velocity != 0.0 {
                    ui.small("当前保留旧惯性参数；调整空气阻力或地面摩擦后，改用加速度减速。");
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("渲染");
                let mut selected_backend = self.backend;
                egui::ComboBox::from_label("渲染后端")
                    .selected_text(selected_backend.label())
                    .show_ui(ui, |ui| {
                        for &backend in RendererBackend::available() {
                            ui.selectable_value(&mut selected_backend, backend, backend.label());
                        }
                    });
                if selected_backend != self.backend {
                    match selected_backend.save() {
                        Ok(()) => {
                            self.pending_backend = Some(selected_backend);
                            self.switch_viewport = Some(ui.ctx().input(|input| {
                                let viewport = input.viewport();
                                let mut builder = egui::ViewportBuilder::default();
                                if let Some(rect) = viewport.outer_rect {
                                    builder = builder.with_position(rect.min);
                                }
                                if let Some(rect) = viewport.inner_rect {
                                    builder = builder.with_inner_size(rect.size());
                                }
                                builder.with_maximized(viewport.maximized.unwrap_or(false))
                            }));
                            ui.ctx().send_viewport_cmd_to(
                                egui::ViewportId::ROOT,
                                egui::ViewportCommand::Close,
                            );
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                egui::ComboBox::from_label("帧率上限")
                    .selected_text(self.prefs.fps.to_string())
                    .show_ui(ui, |ui| {
                        for fps in [30, 60, 120] {
                            ui.selectable_value(&mut self.prefs.fps, fps, fps.to_string());
                        }
                    });
                let mut opacity_changed = ui
                    .add(
                        egui::Slider::new(&mut self.prefs.config.opacity_normal, 0.1..=1.0)
                            .text("常规不透明度"),
                    )
                    .changed();
                opacity_changed |= ui
                    .add(
                        egui::Slider::new(&mut self.prefs.config.opacity_dim, 0.1..=1.0)
                            .text("透明模式不透明度"),
                    )
                    .on_hover_text("只在桌宠鼠标穿透时使用；实际不透明度不超过常规值。")
                    .changed();
                ui.add_space(12.0);
                ui.heading("高亮描边");
                let condition_label = |condition| match condition {
                    OutlineCondition::Never => "关闭",
                    OutlineCondition::Dragging => "拖拽时",
                    OutlineCondition::Pressing => "鼠标按下时",
                    OutlineCondition::Focused => "窗口获得焦点时",
                    OutlineCondition::Always => "始终",
                };
                let outline = &mut self.prefs.config.rendering.outline;
                for (label, condition) in [
                    ("开启条件", &mut outline.condition),
                    ("开启条件（操控中）", &mut outline.controlled_condition),
                ] {
                    egui::ComboBox::from_label(label).selected_text(condition_label(*condition)).show_ui(ui, |ui| {
                        for value in [OutlineCondition::Never, OutlineCondition::Dragging, OutlineCondition::Pressing, OutlineCondition::Focused, OutlineCondition::Always] {
                            opacity_changed |= ui.selectable_value(condition, value, condition_label(value)).changed();
                        }
                    });
                }
                ui.small("操控中指手动动作模式；窗口焦点不等于鼠标悬停。");
                ui.horizontal(|ui| {
                    opacity_changed |= ui.color_edit_button_srgba_unmultiplied(&mut outline.color).changed();
                    ui.label("描边颜色");
                    opacity_changed |= ui.color_edit_button_srgba_unmultiplied(&mut outline.controlled_color).changed();
                    ui.label("描边颜色（操控中）");
                });
                opacity_changed |= ui.add(egui::Slider::new(&mut outline.width, 1.0..=5.0).step_by(0.5)
                    .text("描边宽度").suffix(" 逻辑像素")).changed();
                ui.add_space(12.0);
                ui.heading("光照效果");
                opacity_changed |= ui.add(egui::Slider::new(&mut self.prefs.config.rendering.shadow_opacity, 0.0..=1.0)
                    .text("阴影强度")).on_hover_text("角色轮廓周围的黑色阴影；0 为关闭，不是地面投影。").changed();
                ui.horizontal(|ui| {
                    for (label, value) in [("禁用", 0.0), ("轻微", 119.0/255.0), ("标准", 187.0/255.0), ("重墨", 1.0)] {
                        opacity_changed |= ui.selectable_value(&mut self.prefs.config.rendering.shadow_opacity, value, label).changed();
                    }
                });
                opacity_changed |= ui.checkbox(&mut self.prefs.config.rendering.high_quality, "高质量滤波（描边与阴影）")
                    .on_hover_text("开启使用高斯邻域滤波，关闭使用圆周采样；只影响描边与阴影。").changed();
                opacity_changed |= ui.checkbox(&mut self.prefs.config.rendering.mipmaps, "MipMap 抗锯齿")
                    .on_hover_text("保留原图并生成缩小采样层，减少缩放时的锯齿和闪烁；关闭后使用基础层线性采样。").changed();
                ui.add_space(12.0);
                ui.heading("窗口边界");
                let canvas = &mut self.prefs.config.rendering.canvas;
                let coverage_label = |value: f32| match value {
                    0.45 => "最宽", 0.65 => "较宽", 0.8 => "标准", 0.9 => "较窄", 0.95 => "最窄", _ => "自定义",
                };
                egui::ComboBox::from_label("边界大小").selected_text(coverage_label(canvas.coverage)).show_ui(ui, |ui| {
                    for value in [0.45, 0.65, 0.8, 0.9, 0.95] { ui.selectable_value(&mut canvas.coverage, value, coverage_label(value)); }
                }).response.on_hover_text("根据所有动画采样的累计不透明度裁剪；阈值越高，边界通常越窄。攻击和技能会额外预留挥动范围，避免武器被裁切。");
                let sampling_label = |value| match value { 1 => "极精确", 4 => "精确", 16 => "粗略", 64 => "极粗略", _ => "自定义" };
                egui::ComboBox::from_label("边界计算方式").selected_text(sampling_label(canvas.sampling_interval)).show_ui(ui, |ui| {
                    for value in [1, 4, 16, 64] { ui.selectable_value(&mut canvas.sampling_interval, value, sampling_label(value)); }
                }).response.on_hover_text("累计不透明度以 30 FPS 为基准，分别每 1、4、16、64 帧抽样一次；攻击和技能范围独立细采样。只在载入或重新计算时执行。");
                ui.small("修改后重新计算；不会重置当前动画或暂停状态。");
                if ui.add_enabled(self.renderer.is_some(), egui::Button::new("重新计算边界")).clicked() {
                    if let Some(renderer) = &mut self.renderer {
                        match renderer.recalculate_bounds(*canvas) {
                            Ok(()) => {
                                self.error = None;
                                if let Some(motion) = &mut self.desktop_motion { motion.next_work_area_update = Instant::now(); }
                                opacity_changed = true;
                            }
                            Err(error) => self.error = Some(format!("计算窗口边界失败: {error}")),
                        }
                    }
                }
                if let Some(renderer) = &self.renderer {
                    if renderer.canvas_settings != *canvas { ui.label("边界设置待应用"); }
                    ui.small(format!("已采样 {} 个姿态", renderer.canvas_samples));
                    if renderer.effective_canvas_coverage < renderer.canvas_settings.coverage {
                        ui.small(format!("模型内容较淡，实际裁剪阈值为 {:.1}%", renderer.effective_canvas_coverage * 100.0));
                    }
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("过渡");
                let preset_label = |preset| match preset {
                    TransitionPreset::Disabled => "禁用（0 秒）",
                    TransitionPreset::Fast => "快速（0.1 秒）",
                    TransitionPreset::Standard => "标准（0.3 秒）",
                    TransitionPreset::Slow => "慢速（0.6 秒）",
                };
                for (label, duration, help) in [
                    ("动画交叉过渡", &mut self.prefs.config.animation_mix_duration,
                        "控制不同骨骼动作间的混合；0 为立即切换，下次切换起生效。"),
                    ("常规属性过渡", &mut self.prefs.config.property_transition_duration,
                        "控制位置、朝向翻转、不透明度和描边强度；0 为立即切换。拖拽位置直接跟手，暂停时属性过渡仍会完成。"),
                ] {
                    let selected = TransitionPreset::from_seconds(*duration)
                        .map(|preset| preset_label(preset).to_owned())
                        .unwrap_or_else(|| format!("{duration} 秒（自定义）"));
                    egui::ComboBox::from_label(label).selected_text(selected).show_ui(ui, |ui| {
                        for preset in TransitionPreset::ALL {
                            opacity_changed |= ui.selectable_value(duration, preset.seconds(), preset_label(preset)).changed();
                        }
                    }).response.on_hover_text(help);
                }
                let curve_label = |curve| match curve {
                    EasingFunction::Linear => "线性",
                    EasingFunction::EaseOutSine => "正弦缓出",
                    EasingFunction::EaseOutCubic => "三次缓出",
                    EasingFunction::EaseOutQuint => "五次缓出",
                };
                egui::ComboBox::from_label("缓动函数")
                    .selected_text(curve_label(self.prefs.config.easing))
                    .show_ui(ui, |ui| {
                        for curve in [
                            EasingFunction::Linear,
                            EasingFunction::EaseOutSine,
                            EasingFunction::EaseOutCubic,
                            EasingFunction::EaseOutQuint,
                        ] {
                            opacity_changed |= ui
                                .selectable_value(
                                    &mut self.prefs.config.easing,
                                    curve,
                                    curve_label(curve),
                                )
                                .changed();
                        }
                    });
                if opacity_changed {
                    ui.ctx().request_repaint_of(desktop_id());
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("桌面窗口");
                if ui
                    .checkbox(&mut self.prefs.click_through, "透明模式（鼠标穿透）")
                    .changed()
                {
                    ui.ctx().request_repaint_of(desktop_id());
                }
                let taskbar_changed = ui
                    .add_enabled(
                        shell_visibility::SUPPORTED,
                        egui::Checkbox::new(
                            &mut self.prefs.show_in_taskbar,
                            shell_visibility::label(),
                        ),
                    )
                    .changed();
                let mut area_changed = ui
                    .add_enabled(
                        cfg!(windows),
                        egui::Checkbox::new(
                            &mut self.prefs.auto_taskbar_height,
                            "自动检测任务栏高度",
                        ),
                    )
                    .changed();
                area_changed |= ui
                    .add_enabled(
                        self.prefs.bottom_height().is_some(),
                        egui::Slider::new(&mut self.prefs.taskbar_height, 0..=500)
                            .text("任务栏高度（下边界距离）")
                            .suffix(" px"),
                    )
                    .changed();
                area_changed |= ui
                    .checkbox(&mut self.prefs.multi_monitors, "启用多显示屏")
                    .on_hover_text(
                        "开启后可拖到其他显示器，并可在相接的屏幕间行走；关闭后限制在主显示器。",
                    )
                    .changed();
                ui.add_space(12.0);
                ui.label("初始部署位置");
                ui.small("相对主显示器可活动范围；重新打开桌宠时生效。");
                for (axis, label) in ["水平位置（左 → 右）", "垂直位置（上 → 下）"]
                    .into_iter()
                    .enumerate()
                {
                    let mut percent = self.prefs.initial_position[axis] * 100.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut percent, 0.0..=100.0)
                                .text(label)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        self.prefs.initial_position[axis] = percent / 100.0;
                    }
                }
                if area_changed {
                    if let Some(motion) = &mut self.desktop_motion {
                        motion.next_work_area_update = Instant::now();
                    }
                }
                if taskbar_changed || area_changed {
                    ui.ctx().request_repaint_of(desktop_id());
                }
                ui.add_space(20.0);
                if ui
                    .button(format!("{}  恢复默认参数", icons::ARROW_COUNTER_CLOCKWISE))
                    .clicked()
                {
                    self.prefs.config = PetConfig::default();
                    if let Some(renderer) = &mut self.renderer {
                        if let Err(error) = renderer.recalculate_bounds(self.prefs.config.rendering.canvas) {
                            self.error = Some(format!("恢复默认边界失败: {error}"));
                        }
                    }
                    self.prefs.fps = 60;
                    let defaults = Preferences::default();
                    self.prefs.click_through = defaults.click_through;
                    self.prefs.show_in_taskbar = defaults.show_in_taskbar;
                    self.prefs.auto_taskbar_height = defaults.auto_taskbar_height;
                    self.prefs.taskbar_height = defaults.taskbar_height;
                    self.prefs.multi_monitors = defaults.multi_monitors;
                    self.prefs.initial_position = defaults.initial_position;
                    if let Some(motion) = &mut self.desktop_motion {
                        motion.next_work_area_update = Instant::now();
                    }
                    ui.ctx().request_repaint_of(desktop_id());
                }
            });
        });
    }

    fn import(&mut self, ctx: &egui::Context) {
        if !self.import_open {
            return;
        }
        let mut open = true;
        egui::Window::new("导入本地模型")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label("模型清单 JSON 或 Spine .skel 路径");
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_path).desired_width(f32::INFINITY),
                );
                if ui.button(format!("{}  载入", icons::FOLDER_OPEN)).clicked() {
                    let path = PathBuf::from(self.import_path.trim());
                    let result = if path.extension().is_some_and(|ext| ext == "skel") {
                        let id = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let model = CharacterModel {
                            id: id.clone(),
                            name: id,
                            skeleton: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            atlas: path
                                .with_extension("atlas")
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            texture: path
                                .with_extension("png")
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            ..Default::default()
                        };
                        Ok(ModelCatalog::new(vec![model]))
                    } else {
                        ModelCatalog::from_json_file(&path)
                    };
                    match result {
                        Ok(catalog) if !catalog.models.is_empty() => {
                            let root = path.parent().unwrap_or(Path::new(".")).to_owned();
                            match forms::Forms::load(
                                &catalog,
                                0,
                                &root,
                                self.prefs.config.rendering.canvas,
                                &self.prefs.form_preferences,
                            ) {
                                Ok((forms, renderer)) => {
                                    let active = forms.active;
                                    let model = catalog.models[active].clone();
                                    self.catalog = catalog;
                                    self.asset_root = root;
                                    self.install_model(active, model, renderer);
                                    self.forms = Some(forms);
                                    self.import_open = false;
                                }
                                Err(error) => self.error = Some(error),
                            }
                        }
                        Ok(_) => self.error = Some("清单没有包含任何模型".into()),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            });
        self.import_open &= open;
    }

    fn desktop_builder(&mut self, ctx: &egui::Context) -> Option<egui::ViewportBuilder> {
        if !self.desktop_open {
            if self.keyboard.owner() == Some(desktop_id()) {
                self.cancel_keyboard();
            }
            #[cfg(target_os = "macos")]
            if self.desktop_motion.is_some() {
                ctx.send_viewport_cmd_to(desktop_id(), egui::ViewportCommand::Visible(false));
            }
            self.desktop_motion = None;
            self.desktop_opacity = None;
            self.desktop_presentation = transition::Presentation::default();
            self.desktop_effects = effects::EffectState::default();
            #[cfg(target_os = "macos")]
            return self.desktop_viewport_builder(ctx, None);
            #[cfg(not(target_os = "macos"))]
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let layout = renderer.desktop_layout(
            180.0 * self.prefs.config.display_scale,
            effects::padding(&self.prefs.config),
            self.desktop_motion
                .as_ref()
                .map_or(ctx.pixels_per_point(), |motion| motion.pixels_per_point),
        );
        let body_size = layout.body.size();
        #[cfg(target_os = "macos")]
        let opening = self.desktop_motion.is_none();
        if self.desktop_motion.is_none() {
            let pixels_per_point = ctx.pixels_per_point();
            let motion = desktop::movement_areas(
                ctx,
                pixels_per_point,
                self.prefs.bottom_height(),
                self.prefs.multi_monitors,
            )
            .and_then(|(areas, primary)| {
                desktop::DesktopMotion::deploy(
                    areas,
                    primary,
                    body_size,
                    self.prefs.initial_position,
                    pixels_per_point,
                )
            });
            match motion {
                Ok(motion) => self.desktop_motion = Some(motion),
                Err(error) => {
                    self.error = Some(format!("读取桌面工作区失败: {error}"));
                    self.desktop_open = false;
                    return None;
                }
            }
        }
        let motion = self.desktop_motion.as_ref().unwrap();
        #[cfg(target_os = "macos")]
        if opening {
            // A reused hidden viewport does not receive a UI callback until it is
            // visible again. Wake it here for both toolbar and menu requests.
            ctx.send_viewport_cmd_to(
                desktop_id(),
                egui::ViewportCommand::InnerSize(layout.window_size),
            );
            ctx.send_viewport_cmd_to(
                desktop_id(),
                egui::ViewportCommand::OuterPosition(
                    motion.screen_position() - layout.body.min.to_vec2(),
                ),
            );
            ctx.send_viewport_cmd_to(desktop_id(), egui::ViewportCommand::Visible(true));
            ctx.request_repaint_of(desktop_id());
        }
        self.desktop_viewport_builder(
            ctx,
            Some(
                motion
                    .last_window_position
                    .unwrap_or(motion.screen_position()),
            ),
        )
    }

    fn desktop_viewport_builder(
        &self,
        ctx: &egui::Context,
        position: Option<Pos2>,
    ) -> Option<egui::ViewportBuilder> {
        let renderer = self.renderer.as_ref()?;
        let layout = renderer.desktop_layout(
            180.0 * self.prefs.config.display_scale,
            effects::padding(&self.prefs.config),
            self.desktop_motion
                .as_ref()
                .map_or(ctx.pixels_per_point(), |motion| motion.pixels_per_point),
        );
        let builder = egui::ViewportBuilder::default()
            .with_title("ArkPets - 桌宠")
            .with_visible(false)
            .with_decorations(false)
            .with_has_shadow(false)
            .with_transparent(true)
            // Native styles control taskbar visibility without recreating the viewport.
            .with_taskbar(false)
            .with_inner_size(layout.window_size)
            .with_resizable(false)
            .with_window_level(if self.prefs.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            })
            .with_mouse_passthrough(self.prefs.click_through);
        Some(if let Some(position) = position {
            builder.with_position(position - layout.body.min.to_vec2())
        } else {
            builder
        })
    }

    fn desktop_frame(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        let started = Instant::now();
        let simulation_due = started >= self.next_frame;
        let Some(motion) = self.desktop_motion.as_mut() else {
            return;
        };
        #[cfg(windows)]
        if !motion.appearance_ready {
            match platform::prepare_desktop_window(self.prefs.show_in_taskbar) {
                Ok(platform::DesktopAppearance::Applied) => {}
                Ok(platform::DesktopAppearance::Unsupported) => {
                    eprintln!("Windows does not support the optional border appearance attributes")
                }
                Err(error) => {
                    eprintln!("Desktop window preparation failed: {error}");
                    self.error = Some(error);
                    self.desktop_open = false;
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                    return;
                }
            }
            motion.appearance_ready = true;
            motion.taskbar_visibility = Some(self.prefs.show_in_taskbar);
        }
        if motion.taskbar_visibility != Some(self.prefs.show_in_taskbar) {
            if let Err(error) = shell_visibility::set_visible(self.prefs.show_in_taskbar) {
                self.error = Some(error);
                self.desktop_open = false;
                ctx.request_repaint_of(egui::ViewportId::ROOT);
                return;
            }
            motion.taskbar_visibility = Some(self.prefs.show_in_taskbar);
        }
        let dt = if simulation_due {
            let dt = started
                .duration_since(motion.last_tick)
                .as_secs_f32()
                .min(0.1);
            motion.last_tick = started;
            dt
        } else {
            0.0
        };
        self.advance(&ctx, dt);
        self.handle_keyboard(&ctx);
        let paused = self.paused();
        let (opacity, opacity_animating) = self
            .desktop_opacity
            .get_or_insert_with(|| {
                opacity::OpacityFade::new(&self.prefs.config, self.prefs.click_through, started)
            })
            .update(&self.prefs.config, self.prefs.click_through, started);
        let bottom_height = self.prefs.bottom_height();
        let motion = self.desktop_motion.as_mut().unwrap();
        if self.prefs.click_through {
            motion.body.end_drag();
            motion.gesture = DirectionGesture::default();
        }
        let (Some(runtime), Some(renderer)) = (&mut self.runtime, &self.renderer) else {
            return;
        };
        let character = runtime.character_mut();
        let padding = effects::padding(&self.prefs.config);
        let pixels_per_point = ctx.pixels_per_point();
        let layout = renderer.desktop_layout(
            180.0 * self.prefs.config.display_scale,
            padding,
            pixels_per_point,
        );
        let body_size = layout.body.size();
        let canvas_offset = layout.body.min.to_vec2();
        if ctx.input(|input| {
            input.viewport().inner_rect.is_some_and(|rect| {
                (rect.size() - layout.window_size).abs().max_elem() > 0.5 / pixels_per_point
            })
        }) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(layout.window_size));
        }
        if started >= motion.next_work_area_update
            || motion.pixels_per_point != pixels_per_point
            || motion.size != body_size
        {
            let old_scale = motion.pixels_per_point;
            if let Some(profile) = &self.profile {
                profile
                    .record_desktop_geometry(layout.body, pixels_per_point)
                    .expect("write diagnostic desktop geometry");
            }
            motion.next_work_area_update = started + Duration::from_secs(1);
            if let Err(error) = desktop::movement_areas(
                &ctx,
                pixels_per_point,
                bottom_height,
                self.prefs.multi_monitors,
            )
            .and_then(|(areas, primary)| {
                motion.set_areas(areas, body_size, pixels_per_point)?;
                motion.finish_deployment(primary, body_size);
                Ok(())
            }) {
                self.error = Some(format!("读取桌面工作区失败: {error}"));
                self.desktop_open = false;
                ctx.request_repaint_of(egui::ViewportId::ROOT);
                return;
            }
            let factor = old_scale / pixels_per_point;
            self.desktop_presentation.rescale_position(factor);
            if let Some(previous) = motion.last_window_position {
                let previous = previous * factor;
                motion.last_window_position = Some(previous);
                let constrained = motion.constrain_display_position(previous, body_size);
                if constrained.distance(previous) > 0.001 {
                    self.desktop_presentation
                        .reset_position(constrained, started);
                }
            } else {
                self.desktop_presentation
                    .reset_position(motion.screen_position(), started);
            }
        }
        let mut interacted = false;
        let mut properties_animating = false;
        let was_dragging = motion.body.dragging;
        #[cfg(debug_assertions)]
        if let Some(capture) = &mut self.capture {
            capture.desktop(&ctx, self.frame_ms, layout.body, motion.work_area.bottom());
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            self.desktop_open = false;
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        let window_origin = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .map_or(motion.screen_position() - canvas_offset, |rect| rect.min)
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                let target = layout.paint;
                let response = ui.interact(
                    target,
                    keyboard::target_id(desktop_id()),
                    Sense::click_and_drag(),
                );
                keyboard::focus_response(&response);
                if response.clicked() && !self.prefs.click_through {
                    interacted |= character.click();
                }
                if response.drag_started()
                    && self.prefs.config.allow_drag
                    && !self.prefs.click_through
                {
                    let fallback = response
                        .interact_pointer_pos()
                        .map(|p| window_origin + p.to_vec2());
                    match desktop::cursor_position(pixels_per_point, ctx.zoom_factor(), fallback) {
                        Ok(Some(cursor)) => {
                            motion.start_drag_from_window(
                                cursor,
                                window_origin + canvas_offset,
                                body_size,
                            );
                            if self.prefs.config.allow_interact {
                                character.play_idle();
                                interacted = true;
                            }
                        }
                        Err(error) => {
                            self.error = Some(format!("读取鼠标位置失败: {error}"));
                            ctx.request_repaint_of(egui::ViewportId::ROOT);
                        }
                        Ok(None) => {}
                    }
                }
                if motion.body.dragging || (response.hovered() && !self.prefs.click_through) {
                    let fallback = ctx
                        .input(|input| input.pointer.latest_pos())
                        .map(|p| window_origin + p.to_vec2());
                    match desktop::cursor_position(pixels_per_point, ctx.zoom_factor(), fallback) {
                        Ok(cursor) => {
                            if let Some(direction) = motion.gesture.update(
                                cursor.map(|p| p.x * pixels_per_point),
                                response.hovered(),
                                &self.prefs.config,
                            ) {
                                character.set_direction(direction);
                            }
                            if motion.body.dragging {
                                if !self.prefs.config.allow_drag {
                                    motion.body.end_drag();
                                } else if let Some(cursor) = cursor {
                                    motion.drag_to(
                                        cursor,
                                        ctx.input(|input| input.unstable_dt),
                                        body_size,
                                        self.prefs.config.physics.max_velocity,
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            motion.body.end_drag();
                            motion.gesture = DirectionGesture::default();
                            self.error = Some(format!("读取鼠标位置失败: {error}"));
                            ctx.request_repaint_of(egui::ViewportId::ROOT);
                        }
                    }
                } else {
                    motion.gesture.update(None, false, &self.prefs.config);
                }
                if motion.body.dragging
                    && (response.drag_stopped() || !ctx.input(|input| input.pointer.primary_down()))
                {
                    motion.body.end_drag();
                    if let Some(direction) = motion.gesture.release(&self.prefs.config) {
                        character.set_direction(direction);
                    }
                }
                let (facing, turning) = self.desktop_presentation.facing(
                    character.animation().direction,
                    &self.prefs.config,
                    started,
                );
                properties_animating |= turning;
                let (effect, outline_animating) = self.desktop_effects.update(
                    &self.prefs.config,
                    effects::Input {
                        dragging: motion.body.dragging,
                        pressing: response.is_pointer_button_down_on(),
                        focused: ctx.input(|input| input.viewport().focused.unwrap_or(false)),
                        controlled: !self.prefs.automatic,
                    },
                    opacity,
                    pixels_per_point,
                    started,
                );
                properties_animating |= outline_animating;
                renderer.paint_with_facing(ui.painter(), target, facing, effect, &self.compositor);
                response.context_menu(|ui| {
                    if ui.button("关闭桌宠").clicked() {
                        self.desktop_open = false;
                        ctx.request_repaint_of(egui::ViewportId::ROOT);
                        ui.close();
                    }
                });
            });
        if !motion.body.dragging && self.keyboard.prioritize_walk(character) {
            interacted = false;
        }
        let mut direction = character.animation().direction;
        if !paused && simulation_due {
            if let Some(control) = self.keyboard.walking().filter(|_| character.is_walking()) {
                motion.step_controlled(dt, body_size, &self.prefs.config, control);
                direction = control.direction;
            } else {
                motion.step(
                    dt,
                    body_size,
                    &self.prefs.config,
                    character.is_walking(),
                    &mut direction,
                );
            }
            character.set_direction(direction);
        }
        // Avoid generating an OS move/redraw event for an unchanged pixel.
        let scale = ctx.pixels_per_point();
        let snap_position = motion.body.dragging
            || was_dragging
            || self.keyboard.walking().is_some()
            || self.prefs.config.property_transition_duration == 0.0;
        let (visual_target, reset) = match motion.display_target(body_size, snap_position) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.to_string());
                self.desktop_open = false;
                ctx.request_repaint_of(egui::ViewportId::ROOT);
                return;
            }
        };
        if reset {
            if let Some(previous) = motion.last_window_position {
                self.desktop_presentation.reset_position(previous, started);
            }
        }
        let (visual_position, moving) = self.desktop_presentation.position(
            visual_target,
            snap_position,
            &self.prefs.config,
            started,
        );
        properties_animating |= moving;
        let position =
            match motion.display_position(visual_position, body_size, scale, snap_position) {
                Ok(position) => position,
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.desktop_open = false;
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                    return;
                }
            };
        properties_animating |= motion.last_window_position != Some(position)
            && position.distance(motion.screen_position()) > 1.0 / scale;
        let first_frame = motion.last_window_position.is_none();
        let native_position = position - canvas_offset;
        if motion.last_window_position != Some(position)
            || window_origin.distance(native_position) > 0.5 / pixels_per_point
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(native_position));
            motion.last_window_position = Some(position);
        }
        if first_frame {
            // eframe applies viewport commands after presenting this frame in both
            // backends. Keep the HWND hidden until its pixels and frame are ready.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        if !paused || opacity_animating || properties_animating {
            request_next_frame(&ctx, &mut self.next_frame, self.prefs.fps, started);
        }
        request_repaint_at(&ctx, motion.next_work_area_update);
        if let Some(profile) = &self.profile {
            profile.desktop_callback_finished(!paused && simulation_due);
            if profile.finish_if_ready().expect("write performance report") {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        }
        if interacted {
            self.sync_animation(true);
        }
    }
}

impl App {
    fn preview_visible(&self, ctx: &egui::Context) -> bool {
        self.page == 0
            && self.renderer.is_some()
            && ctx.input(|input| {
                input
                    .raw
                    .viewports
                    .get(&egui::ViewportId::ROOT)
                    .is_some_and(|viewport| viewport.visible() != Some(false))
            })
    }

    #[cfg(target_os = "macos")]
    fn handle_menu_requests(&mut self, ctx: &egui::Context, requests: menu_bar::Requests) {
        if requests.quit {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            return;
        }
        if let Some(open) = requests.desktop_open {
            if !open || self.renderer.is_some() {
                self.desktop_open = open;
                self.desktop_builder(ctx);
            }
        }
        if requests.show_launcher {
            for command in [
                egui::ViewportCommand::Minimized(false),
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Focus,
            ] {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, command);
            }
        }
    }

    fn update_ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        let update_started = Instant::now();
        if !self.theme_applied {
            self.theme(&ctx);
            self.theme_applied = true;
        }
        #[cfg(debug_assertions)]
        if let Some(mut capture) = self.capture.take() {
            capture.update(self, &ctx);
            self.capture = Some(capture);
        }
        // eframe restores persisted bounds after applying the initial builder.
        // Repair undersized bounds left by an interrupted first launch.
        if let Some(size) = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size())) {
            if size.x < 940.0 || size.y < 640.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    size.x.max(940.0),
                    size.y.max(640.0),
                )));
            }
        }
        let now = Instant::now();
        let dt = if now >= self.preview_next_frame {
            let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
            self.last_frame = now;
            dt
        } else {
            0.0
        };
        self.frame_ms = frame.info().cpu_usage.unwrap_or(0.0) * 1000.0;
        if !self.desktop_open {
            self.advance(&ctx, dt);
        }
        self.sidebar(root_ui);
        self.toolbar(root_ui);
        egui::Panel::bottom("status").show(root_ui, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(Color32::from_rgb(194, 57, 60), error);
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if self.paused() {
                        "已暂停"
                    } else if self.renderer.is_some() {
                        "运行中"
                    } else {
                        "待载入"
                    })
                    .small()
                    .color(ACCENT),
                );
                ui.separator();
                ui.label(RichText::new(&self.gpu).small().color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("CPU {:.2} ms", self.frame_ms))
                            .small()
                            .color(MUTED),
                    );
                });
            });
        });
        self.inspector(root_ui);
        self.preview(root_ui, dt);
        self.import(&ctx);
        if let Some(runtime) = &mut self.runtime {
            if let Err(error) = runtime.apply(PetCommand::Configure(self.prefs.config.clone())) {
                self.error = Some(error.to_string());
            }
            runtime
                .apply(PetCommand::SetAutomatic(self.prefs.automatic))
                .unwrap();
        }
        self.handle_keyboard(&ctx);
        // A visible preview must keep painting even when the desktop callback
        // owns Spine updates. Hidden and paused previews need no animation timer.
        if (!self.paused() || self.preview_properties_animating) && self.preview_visible(&ctx) {
            request_next_frame(&ctx, &mut self.preview_next_frame, self.prefs.fps, now);
        }
        if self.desktop_open && ctx.input(|input| !input.events.is_empty()) {
            ctx.request_repaint_of(desktop_id());
        }
        if let Some(profile) = &self.profile {
            profile.root_update_finished(update_started);
        }
    }
}

impl eframe::App for NativeApp {
    #[cfg(target_os = "macos")]
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let requests = self.menu_bar.take_requests();
        let mut state = self.state.lock().unwrap();
        state.handle_menu_requests(ctx, requests);
        self.menu_bar
            .update(state.desktop_open, state.renderer.is_some());
        drop(state);
        self.sync_dock_visibility();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let builder = {
            let mut state = self.state.lock().unwrap();
            state.update_ui(ui, frame);
            let builder = state.desktop_builder(&ctx);
            #[cfg(target_os = "macos")]
            self.menu_bar
                .update(state.desktop_open, state.renderer.is_some());
            builder
        };
        #[cfg(target_os = "macos")]
        self.sync_dock_visibility();
        if let Some(builder) = builder {
            let state = self.state.clone();
            ctx.show_viewport_deferred(desktop_id(), builder, move |ui, _| {
                state.lock().unwrap().desktop_frame(ui);
            });
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eprintln!("Saving ArkPets state");
        let state = self.state.lock().unwrap();
        eframe::set_value(storage, "preferences", &state.prefs);
        eframe::set_value(storage, "favorites", &state.favorites);
        eprintln!("Saved ArkPets state");
    }

    #[cfg(windows)]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.state.lock().unwrap().compositor.destroy_gl();
        eprintln!("ArkPets exit callback");
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    ui.add_sized(
        [34.0, 34.0],
        egui::Button::new(RichText::new(icon).size(19.0)),
    )
    .on_hover_text(tooltip)
}

fn request_next_frame(ctx: &egui::Context, deadline: &mut Instant, fps: u32, started: Instant) {
    // An event callback can start early and finish after the deadline without
    // advancing animation. Keep that frame due instead of silently skipping it.
    if started < *deadline {
        request_repaint_at(ctx, *deadline);
        return;
    }
    let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    let now = Instant::now();
    if now.saturating_duration_since(*deadline) > period * 4 {
        *deadline = now + period;
    } else {
        while *deadline <= now {
            *deadline += period;
        }
    }
    request_repaint_at(ctx, *deadline);
}

fn request_repaint_at(ctx: &egui::Context, deadline: Instant) {
    // egui subtracts predicted_dt from its delay. Compensate for all deadlines,
    // including slow work-area polling, so early wakeups do not spin until due.
    let prediction = ctx.input(|input| Duration::from_secs_f32(input.predicted_dt));
    ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()) + prediction);
}

fn metadata(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).small().color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(value);
        });
    });
}

fn asset_root() -> PathBuf {
    if let Some(path) = std::env::var_os("ARKPETS_ASSETS") {
        return path.into();
    }
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("assets")));
    if let Some(path) = adjacent.filter(|path| path.join("models.json").is_file()) {
        return path;
    }
    PathBuf::from("assets")
}

pub fn run() -> eframe::Result {
    eprintln!("Starting ArkPets");
    let mut selected =
        RendererBackend::requested().map_err(|e| eframe::Error::AppCreation(e.into()))?;
    let mut state: Option<Arc<Mutex<App>>> = None;
    loop {
        run_backend(selected, &mut state)?;
        let next = state
            .as_ref()
            .and_then(|state| state.lock().unwrap().pending_backend.take());
        let Some(next) = next else {
            break;
        };
        eprintln!(
            "Switching renderer: {} -> {}",
            selected.as_str(),
            next.as_str()
        );
        selected = next;
    }
    eprintln!("ArkPets event loop finished");
    Ok(())
}

fn run_backend(selected: RendererBackend, state: &mut Option<Arc<Mutex<App>>>) -> eframe::Result {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
        return Err(eframe::Error::AppCreation(
            "Linux desktop pets require X11 or XWayland (DISPLAY is not set)".into(),
        ));
    }
    // DirectComposition supplies both viewports' pixels, including their alpha.
    // Set this before HWND creation so an opaque redirection bitmap is never allocated.
    #[cfg(windows)]
    egui_winit::set_windows_no_redirection_bitmap(selected == RendererBackend::Dx12);
    let capture = std::env::var_os("ARKPETS_CAPTURE_DIR")
        .or_else(|| std::env::var_os("ARKPETS_PROFILE_DIR"))
        .map(PathBuf::from);
    let wgpu_options = {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew {
            power_preference: eframe::wgpu::PowerPreference::LowPower,
            ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
        };
        if let Some(backends) = selected.wgpu_backend() {
            setup.instance_descriptor.backends = backends;
        }
        #[cfg(windows)]
        if selected == RendererBackend::Dx12 {
            setup
                .instance_descriptor
                .backend_options
                .dx12
                .presentation_system = eframe::wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual;
            // Both viewports share the UI thread, so acquiring an occluded window's
            // backbuffer must not wait on its frame-latency handle.
            setup
                .instance_descriptor
                .backend_options
                .dx12
                .latency_waitable_object =
                eframe::wgpu::wgt::Dx12UseFrameLatencyWaitableObject::None;
        }
        let device_descriptor = setup.device_descriptor;
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = device_descriptor(adapter);
            descriptor.memory_hints = eframe::wgpu::MemoryHints::MemoryUsage;
            descriptor.required_limits.max_non_sampler_bindings = 4096;
            descriptor
        });
        eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: setup.into(),
            surface: eframe::egui_wgpu::SurfaceConfig {
                present_mode: if selected == RendererBackend::Dx12 {
                    // Without the latency wait, FIFO lets repaint bursts queue GPU work
                    // faster than DXGI presents it, retaining staging buffers and command
                    // allocators. Mailbox replaces stale presents; our deadlines pace animation.
                    eframe::wgpu::PresentMode::Mailbox
                } else {
                    eframe::wgpu::PresentMode::AutoVsync
                },
                ..eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
            },
            ..Default::default()
        }
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_transparent(true)
        .with_inner_size([1180.0, 780.0])
        .with_min_inner_size([940.0, 640.0]);
    if let Some(state) = state {
        if let Some(saved) = state.lock().unwrap().switch_viewport.take() {
            let _ = viewport.patch(saved);
        }
    }
    let options = eframe::NativeOptions {
        // Reuse eframe's event loop after releasing the old windows and GPU resources.
        // Keeping the same process also keeps the launching shell attached until exit.
        run_and_return: true,
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|builder| {
            use egui_winit::winit::platform::macos::{
                ActivationPolicy, EventLoopBuilderExtMacOS as _,
            };
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(|builder| {
            use egui_winit::winit::platform::x11::EventLoopBuilderExtX11 as _;
            // xdg-shell has no global positioning or skip-taskbar requests.
            builder.with_x11();
        })),
        renderer: selected
            .renderer()
            .map_err(|e| eframe::Error::AppCreation(e.into()))?,
        #[cfg(windows)]
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        wgpu_options,
        viewport,
        persist_window: capture.is_none(),
        persistence_path: capture.map(|path| path.join("test-preferences.ron")),
        ..Default::default()
    };
    eframe::run_native(
        "ArkPets",
        options,
        Box::new(|cc| {
            let app = if let Some(app) = state.as_ref() {
                app.lock().unwrap().rebind_renderer(cc, selected)?;
                app.clone()
            } else {
                let app = Arc::new(Mutex::new(App::new(cc, selected)));
                *state = Some(app.clone());
                app
            };
            #[cfg(target_os = "macos")]
            let menu_bar = {
                shell_visibility::initialize_application()?;
                let ctx = cc.egui_ctx.clone();
                menu_bar::MenuBar::new(move || ctx.request_repaint_of(egui::ViewportId::ROOT))?
            };
            Ok(Box::new(NativeApp {
                state: app,
                #[cfg(target_os = "macos")]
                menu_bar,
                #[cfg(target_os = "macos")]
                dock_visible: None,
            }))
        }),
    )
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "source-han".into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/SourceHanSansCN-Regular.otf"
        ))
        .into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("source-han".into());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

fn renderer_name(cc: &eframe::CreationContext<'_>) -> String {
    cc.wgpu_render_state
        .as_ref()
        .map(|state| {
            let info = state.adapter.get_info();
            format!("{} / {:?}", info.name, info.backend)
        })
        .unwrap_or_else(|| {
            #[cfg(windows)]
            if let Some(gl) = &cc.gl {
                use eframe::glow::HasContext;
                return format!("{} / OpenGL", unsafe {
                    gl.get_parameter_string(eframe::glow::RENDERER)
                });
            }
            "GPU unavailable".into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_paired_resource_does_not_replace_the_running_character() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let index = app
            .catalog
            .models
            .iter()
            .position(|model| model.id == "texas2")
            .unwrap();
        let battle = app
            .catalog
            .models
            .iter()
            .position(|model| model.id == "texas2-battle")
            .unwrap();
        app.catalog.models[battle].skeleton = "missing-form-resource.skel".into();
        let selected = app.prefs.selected.clone();
        let animations = app.renderer.as_ref().unwrap().spine.animations.as_ptr();
        let sender = app.runtime.as_ref().unwrap().command_sender();
        app.select(index, &ctx);
        assert!(app
            .error
            .as_ref()
            .is_some_and(|error| error.contains("missing-form-resource")));
        assert_eq!(app.prefs.selected, selected);
        assert_eq!(
            app.renderer.as_ref().unwrap().spine.animations.as_ptr(),
            animations
        );
        sender
            .try_send(PetCommand::SetDirection(Direction::Left))
            .unwrap();
        app.advance(&ctx, 0.0);
        assert_eq!(
            app.runtime
                .as_ref()
                .unwrap()
                .character()
                .animation()
                .direction,
            Direction::Left
        );
    }

    #[test]
    fn explicit_forms_preserve_runtime_feet_velocity_pause_and_preferences() {
        use arkpets::model::ModelForm;
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let index = app
            .catalog
            .models
            .iter()
            .position(|model| model.id == "texas2-battle")
            .unwrap();
        app.select(index, &ctx);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.catalog.character_indices().len(), 7);
        app.set_paused(true);
        app.desktop_open = true;
        app.desktop_builder(&ctx).unwrap();
        let runtime = app.runtime.as_ref().unwrap() as *const _;
        let sender = app.runtime.as_ref().unwrap().command_sender();
        let anchor = |app: &App| {
            let motion = app.desktop_motion.as_ref().unwrap();
            motion.screen_position() + Vec2::new(motion.size.x * 0.5, motion.size.y)
        };
        let motion = app.desktop_motion.as_mut().unwrap();
        motion.finish_deployment(motion.work_area, motion.size);
        motion.body.velocity = PetVec2::new(17.0, 29.0);
        let original = anchor(&app);
        let battle_spine = app.renderer.as_ref().unwrap().spine.animations.as_ptr();
        for (form, animation, unavailable, id) in [
            (ModelForm::Base, "Move", "Skill_2_Loop", "texas2"),
            (ModelForm::Battle, "Skill_2_Loop", "Move", "texas2-battle"),
        ] {
            app.select_form(form).unwrap();
            let character = app.runtime.as_ref().unwrap().character();
            assert_eq!(character.model().id, id);
            assert_eq!(
                character.available_animations(),
                app.renderer.as_ref().unwrap().spine.animations
            );
            assert!(!character
                .available_animations()
                .iter()
                .any(|name| name == unavailable));
            let before = character.animation().clone();
            assert!(app
                .runtime
                .as_mut()
                .unwrap()
                .apply(PetCommand::PlayAnimation(unavailable.into()))
                .is_err());
            assert_eq!(
                *app.runtime.as_ref().unwrap().character().animation(),
                before
            );
            sender
                .try_send(PetCommand::PlayAnimation(animation.into()))
                .unwrap();
            app.advance(&ctx, 0.0);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.forms.as_ref().unwrap().active_form(), Some(form));
            assert!(anchor(&app).distance(original) < 1e-3);
            assert_eq!(
                app.desktop_motion.as_ref().unwrap().body.velocity,
                PetVec2::new(17.0, 29.0)
            );
            assert_eq!(app.runtime.as_ref().unwrap() as *const _, runtime);
            assert!(app.paused());
            assert_eq!(app.prefs.selected, id);
        }
        assert_eq!(
            app.renderer.as_ref().unwrap().spine.animations.as_ptr(),
            battle_spine
        );
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&app.prefs).unwrap()).unwrap();
        assert_eq!(restored.form_preferences, app.prefs.form_preferences);
        app.select(app.catalog.representative(index), &ctx);
        assert_eq!(
            app.forms.as_ref().unwrap().active_form(),
            Some(ModelForm::Battle)
        );
    }

    #[test]
    fn legacy_auto_form_preference_selects_one_skeleton_without_merging_actions() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let index = app
            .catalog
            .models
            .iter()
            .position(|model| model.id == "texas2-battle")
            .unwrap();
        let model = &app.catalog.models[index];
        let preference = serde_json::json!({
            "character_id": model.character_id,
            "skin_id": model.skin_id,
            "mode": "auto",
            "preferred": "battle",
        });
        app.prefs.form_preferences = vec![serde_json::from_value(preference).unwrap()];
        app.select(index, &ctx);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(
            app.forms.as_ref().unwrap().active_form(),
            Some(arkpets::model::ModelForm::Battle)
        );
        let runtime = app.runtime.as_mut().unwrap();
        assert!(runtime
            .apply(PetCommand::PlayAnimation("Move".into()))
            .is_err());
        assert!(runtime.apply(PetCommand::Interact).is_err());
        assert_eq!(runtime.character().animation().animation, "Idle");
    }

    #[test]
    fn manual_keyboard_walk_releases_to_the_mouse_selected_action() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.prefs.automatic = false;
        let runtime = app.runtime.as_mut().unwrap();
        runtime.apply(PetCommand::SetAutomatic(false)).unwrap();
        runtime
            .apply(PetCommand::PlayAnimation("Sit".into()))
            .unwrap();
        let mut frame = eframe::Frame::_new_kittest();
        for pressed in [true, false] {
            let mut input = egui::RawInput {
                focused: true,
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .focused = Some(true);
            input.events.push(egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            ctx.run_ui(input, |ui| app.update_ui(ui, &mut frame))
                .drop_without_applying_deltas();
            assert_eq!(app.animation_name, if pressed { "Move" } else { "Sit" });
        }
    }

    fn test_app(ctx: &egui::Context) -> App {
        // Feed monitor input explicitly, like RawInput below; unit tests do not
        // have a native event loop. Real window geometry is verified by scripts.
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::NULL,
                egui_winit::monitors::MonitorSnapshot {
                    monitors: vec![egui_winit::monitors::MonitorInfo {
                        rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)),
                        primary: true,
                    }],
                    updated: Instant::now(),
                },
            )
        });
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        let mut app = App {
            catalog,
            asset_root: root,
            prefs: Preferences::default(),
            runtime: None,
            renderer: None,
            compositor: renderer::Compositor::Recording,
            forms: None,
            selected: 0,
            search: String::new(),
            favorites: Vec::new(),
            favorites_only: false,
            error: None,
            page: 0,
            import_open: false,
            import_path: String::new(),
            desktop_open: false,
            desktop_motion: None,
            desktop_opacity: None,
            desktop_presentation: transition::Presentation::default(),
            preview_presentation: transition::Presentation::default(),
            preview_properties_animating: false,
            preview_effects: effects::EffectState::default(),
            desktop_effects: effects::EffectState::default(),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            preview_next_frame: Instant::now(),
            preview_direction: Direction::Right,
            preview_gesture: DirectionGesture::default(),
            animation_name: String::new(),
            keyboard: keyboard::Control::default(),
            first_position: true,
            frame_ms: 0.0,
            gpu: String::new(),
            theme_applied: false,
            profile: None,
            backend: RendererBackend::default(),
            pending_backend: None,
            switch_viewport: None,
            #[cfg(debug_assertions)]
            capture: None,
        };
        app.select(0, ctx);
        assert!(app.error.is_none(), "{:?}", app.error);
        app
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn menu_requests_work_without_a_ui_pass_and_preserve_pet_state() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.set_paused(true);
        let spine = &app.renderer.as_ref().unwrap().spine as *const _;
        let input = egui::RawInput::default();
        for open in [true, false, true] {
            let output = ctx.run_logic(&input, |ctx| {
                app.handle_menu_requests(
                    ctx,
                    menu_bar::Requests {
                        desktop_open: Some(open),
                        ..Default::default()
                    },
                );
            });
            assert_eq!(app.desktop_open, open);
            assert!(app.paused());
            assert_eq!(&app.renderer.as_ref().unwrap().spine as *const _, spine);
            assert!(!output
                .viewport_commands
                .contains_key(&egui::ViewportId::ROOT));
            assert!(output.viewport_commands[&desktop_id()]
                .contains(&egui::ViewportCommand::Visible(open)));
        }
        let output = ctx.run_logic(&input, |ctx| {
            app.handle_menu_requests(
                ctx,
                menu_bar::Requests {
                    show_launcher: true,
                    ..Default::default()
                },
            );
        });
        assert!(app.desktop_open && app.paused());
        assert_eq!(
            output.viewport_commands[&egui::ViewportId::ROOT],
            vec![
                egui::ViewportCommand::Minimized(false),
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Focus,
            ]
        );
        let output = ctx.run_logic(&input, |ctx| {
            app.handle_menu_requests(
                ctx,
                menu_bar::Requests {
                    show_launcher: true,
                    desktop_open: Some(false),
                    quit: true,
                },
            );
        });
        assert!(app.desktop_open);
        assert_eq!(
            output.viewport_commands[&egui::ViewportId::ROOT],
            vec![egui::ViewportCommand::Close]
        );
    }

    #[test]
    fn renderer_rebind_preserves_pet_state_and_command_queue() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.page = 1;
        app.search = "pepe".into();
        app.prefs.dark = true;
        app.set_paused(true);
        app.desktop_open = true;
        app.desktop_builder(&ctx).unwrap();
        let motion = app.desktop_motion.as_mut().unwrap();
        motion.body.position = PetVec2::new(400.0, 200.0);
        motion.last_window_position = Some(motion.screen_position());
        #[cfg(windows)]
        {
            motion.appearance_ready = true;
        }
        let sender = app.runtime.as_ref().unwrap().command_sender();
        sender
            .try_send(PetCommand::SetDirection(Direction::Left))
            .unwrap();
        let model = app.prefs.selected.clone();
        let spine = &app.renderer.as_ref().unwrap().spine as *const _;

        for &backend in RendererBackend::available() {
            let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
            app.rebind_renderer(&cc, backend).unwrap();
            assert_eq!(app.page, 1);
            assert_eq!(app.search, "pepe");
            assert_eq!(app.prefs.selected, model);
            assert!(app.prefs.dark && app.paused() && app.desktop_open);
            assert_eq!(&app.renderer.as_ref().unwrap().spine as *const _, spine);
            let motion = app.desktop_motion.as_ref().unwrap();
            assert_eq!(motion.body.position, PetVec2::new(400.0, 200.0));
            assert!(motion.last_window_position.is_none());
            #[cfg(windows)]
            assert!(!motion.appearance_ready);
        }
        let runtime = app.runtime.as_mut().unwrap();
        assert_eq!(runtime.drain_commands(|event| event.result.unwrap()), 1);
        assert_eq!(runtime.character().animation().direction, Direction::Left);
        sender.try_send(PetCommand::SetPaused(false)).unwrap();
        assert_eq!(runtime.drain_commands(|event| event.result.unwrap()), 1);
        assert!(!runtime.paused());
    }

    #[test]
    fn desktop_windows_start_hidden_on_every_open_even_when_paused() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        for paused in [false, true] {
            app.set_paused(paused);
            app.desktop_open = true;
            let builder = app.desktop_builder(&ctx).unwrap();
            assert_eq!(builder.visible, Some(false));
            assert_eq!(builder.transparent, Some(true));
            assert_eq!(builder.decorations, Some(false));

            app.desktop_open = false;
            #[cfg(target_os = "macos")]
            assert_eq!(app.desktop_builder(&ctx).unwrap().visible, Some(false));
            #[cfg(not(target_os = "macos"))]
            assert!(app.desktop_builder(&ctx).is_none());
            assert!(app.desktop_motion.is_none());
        }
    }

    #[test]
    fn taskbar_toggle_keeps_the_viewport_and_paused_pet_state() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.desktop_open = true;
        app.set_paused(true);
        let mut previous = app.desktop_builder(&ctx).unwrap();
        for visible in [true, false, true] {
            let motion = app.desktop_motion.as_mut().unwrap();
            motion.body.position = PetVec2::new(300.0, 250.0);
            motion.body.velocity = PetVec2::new(15.0, -20.0);
            motion.last_window_position = Some(motion.screen_position());
            #[cfg(windows)]
            {
                motion.appearance_ready = true;
            }
            app.prefs.show_in_taskbar = visible;
            let builder = app.desktop_builder(&ctx).unwrap();
            assert!(!previous.patch(builder.clone()).1);
            assert_eq!(builder.taskbar, Some(false));
            assert_eq!(builder.visible, Some(false));
            assert!(app.paused());
            let motion = app.desktop_motion.as_ref().unwrap();
            assert_eq!(motion.body.position, PetVec2::new(300.0, 250.0));
            assert_eq!(motion.body.velocity, PetVec2::new(15.0, -20.0));
            assert_eq!(motion.last_window_position, Some(motion.screen_position()));
            #[cfg(windows)]
            assert!(motion.appearance_ready);
        }
    }

    #[test]
    fn desktop_preferences_load_old_configs_and_round_trip_custom_values() {
        let mut prefs: Preferences = serde_json::from_str(
            r#"{"fps":30,"selected":"pepe","config":{"transitionDuration":0.6}}"#,
        )
        .unwrap();
        assert_eq!(prefs.fps, 30);
        assert!(!prefs.show_in_taskbar);
        assert_eq!(prefs.auto_taskbar_height, cfg!(windows));
        assert!(prefs.multi_monitors);
        assert_eq!(prefs.initial_position, [0.2, 0.2]);
        assert_eq!(prefs.config.activity_level, 4);
        assert_eq!(prefs.config.animation_mix_duration, 0.6);
        assert_eq!(prefs.config.property_transition_duration, 0.3);
        assert_eq!(prefs.config.easing, EasingFunction::EaseOutCubic);
        assert!(!prefs.click_through);
        assert_eq!(
            prefs.config.direction_switching,
            DirectionSwitching::DragRelease
        );
        prefs.multi_monitors = false;
        prefs.initial_position = [0.8, 0.1];
        prefs.config.activity_level = 12;
        prefs.config.direction_switching = DirectionSwitching::Hover;
        prefs.show_in_taskbar = true;
        prefs.auto_taskbar_height = false;
        prefs.taskbar_height = 120;
        prefs.click_through = true;
        prefs.config.opacity_dim = 0.25;
        prefs.config.property_transition_duration = 0.1;
        prefs.config.easing = EasingFunction::EaseOutSine;
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(restored.show_in_taskbar);
        assert!(restored.click_through);
        assert_eq!(restored.config.opacity_dim, 0.25);
        assert_eq!(restored.config.property_transition_duration, 0.1);
        assert_eq!(restored.config.easing, EasingFunction::EaseOutSine);
        assert_eq!(restored.config.animation_mix_duration, 0.6);
        assert_eq!(restored.bottom_height(), Some(120));
        assert!(!restored.multi_monitors);
        assert_eq!(restored.initial_position, [0.8, 0.1]);
        assert_eq!(restored.config.activity_level, 12);
        assert_eq!(
            restored.config.direction_switching,
            DirectionSwitching::Hover
        );
    }

    #[test]
    fn preferences_round_trip_through_actual_eframe_ron_storage() {
        #[derive(Default)]
        struct Storage(std::collections::HashMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Storage::default();
        storage.0.insert("preferences".into(), r#"(config:(allowInteract:false,physics:(gravity:1200.0,drag:0.95,maxVelocity:(x:450.0,y:600.0),minVelocity:7.0,bounceDamping:0.2),transitionDuration:0.6,opacityTransitionDuration:0.1,opacityTransition:EASE_OUT_SINE),fps:30)"#.into());
        let mut prefs: Preferences = eframe::get_value(&storage, "preferences")
            .expect("old RON preferences remain readable");
        assert!(prefs.form_preferences.is_empty());
        assert!(!prefs.config.allow_interact);
        assert!(prefs.config.allow_drag);
        assert_eq!(prefs.config.animation_mix_duration, 0.6);
        assert_eq!(prefs.config.property_transition_duration, 0.1);
        assert_eq!(prefs.config.easing, EasingFunction::EaseOutSine);
        assert_eq!(prefs.config.physics.gravity, 1200.0);
        assert_eq!(prefs.config.physics.drag, 0.95);
        assert_eq!(prefs.config.physics.air_friction, 0.0);
        assert_eq!(prefs.config.physics.ground_friction, 0.0);
        assert_eq!(
            prefs.config.physics.max_velocity,
            PetVec2::new(450.0, 600.0)
        );
        eframe::set_value(&mut storage, "preferences", &prefs);
        let restored: Preferences = eframe::get_value(&storage, "preferences").unwrap();
        assert_eq!(restored.config.physics, prefs.config.physics);
        prefs.config.allow_drag = false;
        prefs.config.physics.air_friction = 150.0;
        prefs.config.physics.ground_friction = 700.0;
        prefs.config.physics.drag = 1.0;
        prefs.config.physics.min_velocity = 0.0;
        prefs.config.animation_mix_duration = 0.137;
        prefs.config.property_transition_duration = 0.423;
        prefs.config.easing = EasingFunction::EaseOutQuint;
        prefs.config.rendering.outline.condition = OutlineCondition::Pressing;
        prefs.config.rendering.outline.controlled_condition = OutlineCondition::Never;
        prefs.config.rendering.outline.color = [12, 34, 56, 78];
        prefs.config.rendering.outline.width = 3.5;
        prefs.config.rendering.shadow_opacity = 0.25;
        prefs.config.rendering.high_quality = false;
        prefs.config.rendering.mipmaps = false;
        prefs.config.rendering.canvas = arkpets::config::CanvasSettings {
            coverage: 0.65,
            sampling_interval: 16,
        };
        prefs.form_preferences.push(forms::Preference {
            character_id: "texas2".into(),
            skin_id: "default".into(),
            mode: forms::Mode::Battle,
            preferred: arkpets::model::ModelForm::Battle,
        });
        eframe::set_value(&mut storage, "preferences", &prefs);
        let restored: Preferences = eframe::get_value(&storage, "preferences").unwrap();
        assert_eq!(restored.form_preferences, prefs.form_preferences);
        assert_eq!(restored.config.physics, prefs.config.physics);
        assert!(!restored.config.allow_drag);
        assert_eq!(restored.config.animation_mix_duration, 0.137);
        assert_eq!(restored.config.property_transition_duration, 0.423);
        assert_eq!(restored.config.easing, EasingFunction::EaseOutQuint);
        assert_eq!(restored.config.rendering, prefs.config.rendering);
        let saved = &storage.0["preferences"];
        assert!(
            saved.contains("animationMixDuration:")
                && saved.contains("propertyTransitionDuration:")
        );
        assert!(!saved.contains("opacityTransitionDuration:"));
    }

    #[test]
    fn early_callback_cannot_consume_an_animation_deadline() {
        let ctx = egui::Context::default();
        let due = Instant::now() - Duration::from_millis(5);
        let mut deadline = due;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            request_next_frame(ui.ctx(), &mut deadline, 60, due - Duration::from_millis(1));
        })
        .drop_without_applying_deltas();
        assert_eq!(deadline, due, "early event skipped a pose update");
        ctx.run_ui(egui::RawInput::default(), |ui| {
            request_next_frame(ui.ctx(), &mut deadline, 60, due);
        })
        .drop_without_applying_deltas();
        assert!(
            deadline > due,
            "consumed animation frame was not rescheduled"
        );
    }

    #[test]
    fn visible_preview_animates_without_pointer_input_with_or_without_a_desktop_pet() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        let mut time = 0.0;
        for (desktop_open, page, paused, minimized, occluded, animated) in [
            (true, 0, false, false, false, true),
            (true, 1, false, false, false, false),
            (true, 0, true, false, false, false),
            (true, 0, false, true, false, false),
            (true, 0, false, false, true, false),
            (false, 0, false, false, false, true),
            (false, 1, false, false, false, false),
            (false, 0, true, false, false, false),
            (false, 0, false, true, false, false),
            (false, 0, false, false, true, false),
        ] {
            app.desktop_open = desktop_open;
            app.page = page;
            app.set_paused(paused);
            // Let egui's initial layout and follow-up repaints settle.
            for pass in 0..5 {
                time += 1.0;
                let mut input = egui::RawInput {
                    time: Some(time),
                    focused: false,
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180.0, 780.0))),
                    ..Default::default()
                };
                let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
                viewport.minimized = Some(minimized);
                viewport.occluded = Some(occluded);
                viewport.focused = Some(false);
                let output = ctx.run_ui(input, |ui| app.update_ui(ui, &mut frame));
                if pass == 4 {
                    assert_eq!(
                        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                            != Duration::MAX,
                        animated,
                        "desktop={desktop_open}, page={page}, paused={paused}"
                    );
                }
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn preview_repaints_reuse_the_desktop_pose() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.desktop_open = true;
        let pose = |app: &mut App| {
            let mut parts = Vec::new();
            app.renderer.as_mut().unwrap().spine.meshes(&mut parts);
            parts
                .into_iter()
                .flat_map(|part| part.positions)
                .collect::<Vec<_>>()
        };
        app.advance(&ctx, 1.0 / 60.0);
        let before = pose(&mut app);
        assert!(!before.is_empty());
        let mut frame = eframe::Frame::_new_kittest();
        for _ in 0..10 {
            app.last_frame = Instant::now() - Duration::from_millis(20);
            app.preview_next_frame = app.last_frame;
            ctx.run_ui(egui::RawInput::default(), |ui| {
                app.update_ui(ui, &mut frame)
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(
            before,
            pose(&mut app),
            "preview advanced the shared skeleton twice"
        );
        app.advance(&ctx, 0.1);
        assert_ne!(
            before,
            pose(&mut app),
            "desktop driver did not advance the pose"
        );
    }

    #[test]
    fn paused_desktop_waits_until_the_work_area_deadline() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.desktop_open = true;
        app.desktop_builder(&ctx).unwrap();
        app.set_paused(true);
        let motion = app.desktop_motion.as_mut().unwrap();
        motion.taskbar_visibility = Some(app.prefs.show_in_taskbar);
        #[cfg(windows)]
        {
            motion.appearance_ready = true;
        }
        motion.last_window_position = Some(motion.screen_position());
        let deadline = Instant::now() + Duration::from_secs(10);
        motion.next_work_area_update = deadline;
        for pass in 0..5 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| app.desktop_frame(ui));
            if pass == 4 {
                let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
                assert!(Instant::now() + delay >= deadline, "early wake: {delay:?}");
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn transparent_mode_fades_real_meshes_while_paused_then_stops_repainting() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.desktop_open = true;
        app.set_paused(true);
        app.prefs.config.opacity_dim = 0.25;
        app.prefs.config.property_transition_duration = 1.0;
        app.runtime
            .as_mut()
            .unwrap()
            .apply(PetCommand::Configure(app.prefs.config.clone()))
            .unwrap();
        app.desktop_builder(&ctx).unwrap();
        let motion = app.desktop_motion.as_mut().unwrap();
        motion.taskbar_visibility = Some(app.prefs.show_in_taskbar);
        #[cfg(windows)]
        {
            motion.appearance_ready = true;
        }
        motion.size = app
            .renderer
            .as_ref()
            .unwrap()
            .desktop_layout(
                180.0,
                effects::padding(&app.prefs.config),
                ctx.pixels_per_point(),
            )
            .body
            .size();
        motion.last_window_position = Some(motion.screen_position());
        let deadline = Instant::now() + Duration::from_secs(10);
        motion.next_work_area_update = deadline;
        let position = motion.body.position;
        let spine = &app.renderer.as_ref().unwrap().spine as *const _;
        let alphas = |output: &egui::FullOutput| -> Vec<u8> {
            output
                .shapes
                .iter()
                .flat_map(|shape| match &shape.shape {
                    egui::Shape::Callback(callback) => callback
                        .callback
                        .downcast_ref::<renderer::RecordedGroup>()
                        .map(|group| {
                            group
                                .meshes
                                .iter()
                                .flat_map(|mesh| mesh.vertices.iter().map(|v| v.color.a()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                })
                .collect()
        };
        let output = ctx.run_ui(Default::default(), |ui| app.desktop_frame(ui));
        let normal = alphas(&output);
        assert!(normal.iter().any(|&a| a > 0));
        output.drop_without_applying_deltas();

        app.prefs.click_through = true;
        assert_eq!(
            app.desktop_builder(&ctx).unwrap().mouse_passthrough,
            Some(true)
        );
        for _ in 0..5 {
            let output = ctx.run_ui(Default::default(), |ui| app.desktop_frame(ui));
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                    < Duration::from_secs(1)
            );
            output.drop_without_applying_deltas();
        }
        app.prefs.config.property_transition_duration = 0.0;
        app.runtime
            .as_mut()
            .unwrap()
            .apply(PetCommand::Configure(app.prefs.config.clone()))
            .unwrap();
        for pass in 0..5 {
            let output = ctx.run_ui(Default::default(), |ui| app.desktop_frame(ui));
            let dim = alphas(&output);
            assert_eq!(normal.len(), dim.len());
            assert_eq!(
                normal, dim,
                "slot alpha must not be reduced before compositing"
            );
            let group = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Callback(callback) => {
                        callback.callback.downcast_ref::<renderer::RecordedGroup>()
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(group.opacity, 0.25);
            if pass == 4 {
                assert!(
                    Instant::now() + output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                        >= deadline
                );
            }
            output.drop_without_applying_deltas();
        }
        assert!(app.paused());
        assert_eq!(app.desktop_motion.as_ref().unwrap().body.position, position);
        assert_eq!(&app.renderer.as_ref().unwrap().spine as *const _, spine);
        app.prefs.click_through = false;
        assert_eq!(
            app.desktop_builder(&ctx).unwrap().mouse_passthrough,
            Some(false)
        );
        let output = ctx.run_ui(Default::default(), |ui| app.desktop_frame(ui));
        assert_eq!(alphas(&output), normal);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn commands_survive_model_switches_and_update_the_render_configuration() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let sender = app.runtime.as_ref().unwrap().command_sender();
        sender.try_send(PetCommand::SetPaused(true)).unwrap();
        app.advance(&ctx, 0.0);
        assert!(app.paused());
        app.select(1, &ctx);
        assert!(app.paused());
        let config = PetConfig {
            display_scale: 1.4,
            walk_speed: 75.0,
            ..Default::default()
        };
        sender.try_send(PetCommand::Configure(config)).unwrap();
        sender
            .try_send(PetCommand::ConfigureTransitions(
                arkpets::TransitionUpdate {
                    animation_mix_duration: Some(0.137),
                    property_transition_duration: Some(0.423),
                    easing: Some(EasingFunction::Linear),
                },
            ))
            .unwrap();
        sender.try_send(PetCommand::SetAutomatic(false)).unwrap();
        sender
            .try_send(PetCommand::PlayAnimation("Move".into()))
            .unwrap();
        sender.try_send(PetCommand::SetPaused(false)).unwrap();
        let mut frame = eframe::Frame::_new_kittest();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            app.update_ui(ui, &mut frame)
        })
        .drop_without_applying_deltas();
        assert!(!app.paused());
        assert_eq!(app.animation_name, "Move");
        assert_eq!(app.prefs.config.display_scale, 1.4);
        assert_eq!(app.prefs.config.animation_mix_duration, 0.137);
        assert_eq!(app.prefs.config.property_transition_duration, 0.423);
        assert_eq!(app.prefs.config.easing, EasingFunction::Linear);
        assert_eq!(
            app.runtime
                .as_ref()
                .unwrap()
                .character()
                .config()
                .walk_speed,
            75.0
        );
        assert!(!app.prefs.automatic);
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn preview_gravity_continues_with_a_separate_desktop_body() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        app.desktop_open = true;
        app.desktop_motion = Some(desktop::DesktopMotion::new(Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(1920.0, 1080.0),
        )));
        let desktop_position = app.desktop_motion.as_ref().unwrap().body.position;
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            ..Default::default()
        };
        ctx.run_ui(input(), |ui| app.preview(ui, 0.0))
            .drop_without_applying_deltas();
        app.runtime
            .as_mut()
            .unwrap()
            .character_mut()
            .set_position(PetVec2::new(50.0, 0.0), Bounds::new(900.0, 700.0));
        for _ in 0..60 {
            ctx.run_ui(input(), |ui| app.preview(ui, 1.0 / 60.0))
                .drop_without_applying_deltas();
        }
        assert!(app.runtime.as_ref().unwrap().character().position().y > 100.0);
        assert_eq!(
            app.desktop_motion.as_ref().unwrap().body.position,
            desktop_position
        );
    }

    #[test]
    fn battle_playback_does_not_infer_transitions_in_either_mode() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        let index = app
            .catalog
            .models
            .iter()
            .position(|model| model.id == "whitw2-battle")
            .unwrap();
        app.select(index, &ctx);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.animation_name, "Idle");
        let renderer = app.renderer.as_ref().unwrap();
        assert!(renderer.bounds.is_finite() && renderer.bounds.is_positive());
        assert!(renderer.triangles > 0);
        let duration = renderer.spine.animation_duration("Attack").unwrap();
        let sender = app.runtime.as_ref().unwrap().command_sender();
        sender.try_send(PetCommand::SetAutomatic(false)).unwrap();
        sender
            .try_send(PetCommand::PlayAnimation("Attack".into()))
            .unwrap();
        for _ in 0..(duration * 60.0 * 2.5).ceil() as usize {
            app.advance(&ctx, 1.0 / 60.0);
        }
        assert_eq!(app.animation_name, "Attack");
        assert!(!app.prefs.automatic);
        assert!(app.error.is_none(), "{:?}", app.error);

        sender.try_send(PetCommand::SetAutomatic(true)).unwrap();
        for _ in 0..(duration * 60.0 * 3.0).ceil() as usize {
            app.advance(&ctx, 1.0 / 60.0);
            assert_eq!(app.animation_name, "Attack");
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn layout_and_pointer_hit_testing_remain_finite() {
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx);
        for size in [
            Vec2::new(1180.0, 780.0),
            Vec2::new(940.0, 640.0),
            Vec2::new(64.0, 64.0),
        ] {
            for y in (0..size.y as usize).step_by(30) {
                for x in (0..size.x as usize).step_by(30) {
                    let input = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events: vec![egui::Event::PointerMoved(Pos2::new(x as f32, y as f32))],
                        ..Default::default()
                    };
                    let output = ctx.run_ui(input, |ui| {
                        app.sidebar(ui);
                        app.toolbar(ui);
                        app.inspector(ui);
                        app.preview(ui, 1.0 / 60.0);
                    });
                    for shape in &output.shapes {
                        let meshes: Vec<_> = match &shape.shape {
                            egui::Shape::Mesh(mesh) => vec![mesh.as_ref()],
                            egui::Shape::Callback(callback) => callback
                                .callback
                                .downcast_ref::<renderer::RecordedGroup>()
                                .map(|group| group.meshes.iter().collect())
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        for mesh in meshes {
                            assert!(
                                mesh.vertices.iter().all(|v| v.pos.is_finite()),
                                "non-finite mesh at {size:?}"
                            );
                        }
                    }
                    output.drop_without_applying_deltas();
                }
            }
        }
        app.desktop_open = true;
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180.0, 780.0))),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                app.desktop_builder(ui.ctx());
            })
            .drop_without_applying_deltas();
        }
        assert!(app.desktop_motion.is_some());
    }
}

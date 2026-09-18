mod backend;
mod desktop;
mod performance;
#[cfg(windows)]
mod platform;
mod renderer;
mod shell_visibility;
#[cfg(debug_assertions)]
mod smoke;

use arkpets::{
    runtime::CommandKind, Bounds, CharacterModel, CharacterState, Direction, ModelCatalog,
    PetCommand, PetConfig, PetRuntime, Vec2 as PetVec2,
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
    last_frame: Instant,
    next_frame: Instant,
    preview_next_frame: Instant,
    preview_direction: Direction,
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
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            preview_next_frame: Instant::now(),
            preview_direction: Direction::Right,
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
            result.page = 1;
        }
        result
    }

    fn rebind_renderer(
        &mut self,
        cc: &eframe::CreationContext<'_>,
        backend: RendererBackend,
    ) -> Result<(), String> {
        configure_fonts(&cc.egui_ctx);
        if let Some(renderer) = &mut self.renderer {
            renderer.rebind_textures(&cc.egui_ctx)?;
        }
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
        let model = self.catalog.models[index].clone();
        match Renderer::load(ctx, &model, &self.asset_root) {
            Ok(renderer) => self.install_model(index, model, renderer),
            Err(error) => self.error = Some(error),
        }
    }

    fn install_model(&mut self, index: usize, mut model: CharacterModel, renderer: Renderer) {
        model.animations = renderer.spine.animations.clone();
        let mut character = CharacterState::new(
            model.clone(),
            self.prefs.config.clone(),
            PetVec2::new(160.0, 220.0),
            42 + index as u64,
        );
        let initial = model
            .animations
            .iter()
            .find(|s| matches!(s.as_str(), "Relax" | "Idle" | "Stand"))
            .unwrap_or(&model.animations[0]);
        character.play_animation(initial.clone());
        self.animation_name = initial.clone();
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
        self.first_position = true;
        self.error = None;
    }

    fn sync_animation(&mut self, force: bool) {
        if let (Some(runtime), Some(renderer)) = (&self.runtime, &mut self.renderer) {
            let character = runtime.character();
            let name = &character.animation().animation;
            if force || name != &self.animation_name {
                match renderer.spine.play(name, name != "Interact") {
                    Ok(()) => self.animation_name = name.clone(),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
    }

    fn paused(&self) -> bool {
        self.runtime.as_ref().is_some_and(PetRuntime::paused)
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some(runtime) = &mut self.runtime {
            runtime.apply(PetCommand::SetPaused(paused)).unwrap();
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
                match renderer.update(dt, self.prefs.config.transition_duration) {
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
                let matching: Vec<usize> = self
                    .catalog
                    .search(&self.search)
                    .iter()
                    .filter(|model| !self.favorites_only || self.favorites.contains(&model.id))
                    .filter_map(|model| self.catalog.models.iter().position(|m| m.id == model.id))
                    .collect();
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 100.0).max(120.0))
                    .show(ui, |ui| {
                        if matching.is_empty() {
                            ui.label(RichText::new("没有匹配的模型").color(MUTED));
                        }
                        for index in matching {
                            let model = &self.catalog.models[index];
                            let title = format!("{}\n{}", model.name, model.id);
                            let button = egui::Button::new(RichText::new(title).size(14.0).color(
                                if index == self.selected && self.renderer.is_some() {
                                    Color32::WHITE
                                } else if self.prefs.dark {
                                    Color32::from_gray(225)
                                } else {
                                    INK
                                },
                            ))
                            .selected(index == self.selected && self.renderer.is_some());
                            if ui.add_sized([202.0, 60.0], button).clicked() {
                                self.select(index, &ctx);
                                self.page = 0;
                            }
                        }
                    });
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("{} 个本地模型", self.catalog.models.len()))
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
        egui::Panel::right("inspector")
            .exact_size(250.0)
            .resizable(false)
            .show(root_ui, |ui| {
                ui.add_space(20.0);
                let Some(character) = self.runtime.as_mut().map(PetRuntime::character_mut) else {
                    ui.label("未载入模型");
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("角色").small().color(MUTED));
                    let id = character.model().id.clone();
                    let favorite = self.favorites.contains(&id);
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
                                self.favorites.retain(|value| value != &id);
                            } else {
                                self.favorites.push(id);
                            }
                        }
                    });
                });
                ui.label(RichText::new(&character.model().name).size(21.0).strong());
                ui.label(RichText::new(&character.model().id).small().color(MUTED));
                ui.add_space(20.0);
                ui.separator();
                ui.add_space(8.0);
                ui.label(RichText::new("动作").strong());
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.prefs.automatic, true, "自动");
                    ui.selectable_value(&mut self.prefs.automatic, false, "手动");
                });
                let mut selected = character.animation().animation.clone();
                egui::ComboBox::from_id_salt("animation")
                    .width(220.0)
                    .selected_text(&selected)
                    .show_ui(ui, |ui| {
                        for animation in character.available_animations() {
                            ui.selectable_value(&mut selected, animation.clone(), animation);
                        }
                    });
                let mut force = false;
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
                    egui::Slider::new(&mut self.prefs.config.opacity_normal, 0.15..=1.0)
                        .text("不透明度"),
                );
                ui.checkbox(&mut self.prefs.always_on_top, "桌宠置顶");
                ui.checkbox(&mut self.prefs.click_through, "桌宠鼠标穿透");
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
                    self.sync_animation(true);
                }
            });
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(root_ui.visuals().panel_fill))
            .show(root_ui, |ui| {
                if self.page == 1 {
                    self.settings(ui);
                    return;
                }
                let available = ui.available_size();
                let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
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
                let mut size = renderer.size(height);
                if size.x > rect.width() - 40.0 {
                    size *= (rect.width() - 40.0).max(1.0) / size.x;
                }
                character.resize(PetVec2::new(size.x, size.y));
                let bounds = Bounds::new(rect.width(), (rect.height() - 70.0).max(size.y));
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
                    character.tick(dt, bounds);
                }
                self.preview_direction = character.animation().direction;
                let p = character.position();
                let target = Rect::from_min_size(rect.min + Vec2::new(p.x, p.y), size);
                let response = ui.interact(target, ui.id().with("pet"), Sense::click_and_drag());
                let mut force = false;
                if response.clicked() {
                    force |= character.click();
                }
                if response.drag_started() {
                    if let Some(point) = ctx.input(|input| input.pointer.press_origin()) {
                        character
                            .start_drag(PetVec2::new(point.x - rect.left(), point.y - rect.top()));
                        force = true;
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
                if response.drag_stopped() {
                    character.end_drag();
                }
                if !character.is_hidden() {
                    renderer.paint(
                        &painter,
                        target,
                        character.animation().direction,
                        self.prefs.config.opacity_normal,
                    );
                }
                if self.desktop_open {
                    character.set_direction(desktop_direction);
                }
                painter.text(
                    rect.center_bottom() - Vec2::new(0.0, 28.0),
                    egui::Align2::CENTER_BOTTOM,
                    &character.model().name,
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
                ui.checkbox(&mut self.prefs.config.allow_interact, "允许触摸和拖拽");
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.walk_speed, 0.0..=120.0)
                        .text("行走速度"),
                );
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("物理");
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.gravity, 0.0..=2000.0)
                        .text("重力"),
                );
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.drag, 0.8..=1.0)
                        .text("惯性保留"),
                );
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.physics.bounce_damping, 0.0..=1.0)
                        .text("反弹系数"),
                );
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
                ui.add(
                    egui::Slider::new(&mut self.prefs.config.transition_duration, 0.0..=1.0)
                        .text("动作混合 / 秒"),
                );
                egui::ComboBox::from_label("帧率上限")
                    .selected_text(self.prefs.fps.to_string())
                    .show_ui(ui, |ui| {
                        for fps in [30, 60, 120] {
                            ui.selectable_value(&mut self.prefs.fps, fps, fps.to_string());
                        }
                    });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.heading("桌面窗口");
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
                    self.prefs.fps = 60;
                    let defaults = Preferences::default();
                    self.prefs.show_in_taskbar = defaults.show_in_taskbar;
                    self.prefs.auto_taskbar_height = defaults.auto_taskbar_height;
                    self.prefs.taskbar_height = defaults.taskbar_height;
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
                            match Renderer::load(ctx, &catalog.models[0], &root) {
                                Ok(renderer) => {
                                    let model = catalog.models[0].clone();
                                    self.catalog = catalog;
                                    self.asset_root = root;
                                    self.install_model(0, model, renderer);
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
            #[cfg(target_os = "macos")]
            if self
                .desktop_motion
                .as_ref()
                .is_some_and(|motion| motion.taskbar_visibility == Some(false))
            {
                if let Err(error) = shell_visibility::restore_launcher() {
                    self.error = Some(error);
                    return None;
                }
            }
            self.desktop_motion = None;
            return None;
        }
        if self.desktop_motion.is_none() {
            let pixels_per_point = ctx.pixels_per_point();
            let area = ctx.input(|input| {
                let viewport = input.viewport();
                desktop::work_area(
                    viewport.monitor_size.unwrap_or(Vec2::new(1280.0, 720.0)),
                    viewport.outer_rect.map_or(Pos2::ZERO, |r| r.min),
                    pixels_per_point,
                    self.prefs.bottom_height(),
                )
            });
            match area {
                Ok(area) => self.desktop_motion = Some(desktop::DesktopMotion::new(area)),
                Err(error) => {
                    self.error = Some(format!("读取桌面工作区失败: {error}"));
                    self.desktop_open = false;
                    return None;
                }
            }
        }
        let motion = self.desktop_motion.as_mut().unwrap();
        let renderer = self.renderer.as_ref()?;
        let size = renderer.size(180.0 * self.prefs.config.display_scale);
        let window_size = size + Vec2::new(40.0, 20.0);
        let builder = egui::ViewportBuilder::default()
            .with_title("ArkPets - 桌宠")
            .with_visible(false)
            .with_decorations(false)
            .with_has_shadow(false)
            .with_transparent(true)
            // Native styles control taskbar visibility without recreating the viewport.
            .with_taskbar(false)
            .with_inner_size(window_size)
            .with_position(motion.screen_position())
            .with_resizable(false)
            .with_window_level(if self.prefs.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            })
            .with_mouse_passthrough(self.prefs.click_through);
        Some(builder)
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
        let paused = self.paused();
        let bottom_height = self.prefs.bottom_height();
        let motion = self.desktop_motion.as_mut().unwrap();
        let (Some(runtime), Some(renderer)) = (&mut self.runtime, &self.renderer) else {
            return;
        };
        let character = runtime.character_mut();
        let size = renderer.size(180.0 * self.prefs.config.display_scale);
        let window_size = size + Vec2::new(40.0, 20.0);
        if started >= motion.next_work_area_update {
            motion.next_work_area_update = started + Duration::from_secs(1);
            let pixels_per_point = ctx.pixels_per_point();
            let area = ctx.input(|input| {
                desktop::work_area(
                    input
                        .viewport()
                        .monitor_size
                        .unwrap_or(Vec2::new(1280.0, 720.0)),
                    motion.screen_position() + window_size * 0.5,
                    pixels_per_point,
                    bottom_height,
                )
            });
            match area {
                Ok(area) => motion.set_work_area(area, window_size),
                Err(error) => {
                    self.error = Some(format!("读取桌面工作区失败: {error}"));
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
            }
        }
        let mut interacted = false;
        #[cfg(debug_assertions)]
        if let Some(capture) = &mut self.capture {
            capture.desktop(&ctx, self.frame_ms);
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            self.desktop_open = false;
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        let window_origin = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .map_or(motion.screen_position(), |rect| rect.min)
        });
        let pixels_per_point = ctx.pixels_per_point();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                let target = Rect::from_min_size(Pos2::new(20.0, 20.0), size);
                renderer.paint(
                    ui.painter(),
                    target,
                    character.animation().direction,
                    self.prefs.config.opacity_normal,
                );
                let response =
                    ui.interact(target, ui.id().with("desktop-pet"), Sense::click_and_drag());
                if response.clicked() {
                    interacted |= character.click();
                }
                if response.drag_started() && self.prefs.config.allow_interact {
                    let fallback = response
                        .interact_pointer_pos()
                        .map(|p| window_origin + p.to_vec2());
                    match desktop::cursor_position(pixels_per_point, fallback) {
                        Ok(Some(cursor)) => {
                            motion.body.start_drag(motion.local_cursor(cursor));
                            character.play_animation("Relax");
                            interacted = true;
                        }
                        Err(error) => {
                            self.error = Some(format!("读取鼠标位置失败: {error}"));
                            ctx.request_repaint_of(egui::ViewportId::ROOT);
                        }
                        Ok(None) => {}
                    }
                }
                if motion.body.dragging {
                    if simulation_due {
                        let fallback = response
                            .interact_pointer_pos()
                            .map(|p| window_origin + p.to_vec2());
                        match desktop::cursor_position(pixels_per_point, fallback) {
                            Ok(Some(cursor)) => motion.body.drag_to(
                                motion.local_cursor(cursor),
                                dt,
                                Bounds::new(motion.work_area.width(), motion.work_area.height()),
                                PetVec2::new(window_size.x, window_size.y),
                                self.prefs.config.physics.max_velocity,
                            ),
                            Err(error) => {
                                motion.body.end_drag();
                                self.error = Some(format!("读取鼠标位置失败: {error}"));
                                ctx.request_repaint_of(egui::ViewportId::ROOT);
                            }
                            Ok(None) => {}
                        }
                    }
                    if response.drag_stopped() || !ctx.input(|input| input.pointer.primary_down()) {
                        motion.body.end_drag();
                    }
                }
                response.context_menu(|ui| {
                    if ui.button("关闭桌宠").clicked() {
                        self.desktop_open = false;
                        ctx.request_repaint_of(egui::ViewportId::ROOT);
                        ui.close();
                    }
                });
            });
        let mut direction = character.animation().direction;
        if !paused && simulation_due {
            motion.step(
                dt,
                window_size,
                &self.prefs.config,
                character.animation().animation == "Move",
                &mut direction,
            );
            character.set_direction(direction);
        }
        // Avoid generating an OS move/redraw event for an unchanged pixel.
        let scale = ctx.pixels_per_point();
        let position = (motion.screen_position() * scale).round() / scale;
        let first_frame = motion.last_window_position.is_none();
        if motion.last_window_position != Some(position) {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
            motion.last_window_position = Some(position);
        }
        if first_frame {
            // eframe applies viewport commands after presenting this frame in both
            // backends. Keep the HWND hidden until its pixels and frame are ready.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        if !paused {
            request_next_frame(&ctx, &mut self.next_frame, self.prefs.fps);
        }
        ctx.request_repaint_after(
            motion
                .next_work_area_update
                .saturating_duration_since(Instant::now()),
        );
        if let Some(profile) = &self.profile {
            profile.desktop_callback_finished();
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
        if !self.paused() && self.page == 0 {
            request_next_frame(&ctx, &mut self.preview_next_frame, self.prefs.fps);
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let builder = {
            let mut state = self.state.lock().unwrap();
            state.update_ui(ui, frame);
            state.desktop_builder(&ctx)
        };
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

fn request_next_frame(ctx: &egui::Context, deadline: &mut Instant, fps: u32) {
    let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    let now = Instant::now();
    if now.saturating_duration_since(*deadline) > period * 4 {
        *deadline = now + period;
    } else {
        while *deadline <= now {
            *deadline += period;
        }
    }
    // egui subtracts predicted_dt from its delay; compensate so our deadline
    // is an actual frame limit, including on high-refresh-rate displays.
    let prediction = ctx.input(|input| Duration::from_secs_f32(input.predicted_dt));
    ctx.request_repaint_after(deadline.saturating_duration_since(now) + prediction);
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
            Ok(Box::new(NativeApp { state: app }))
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

    fn test_app(ctx: &egui::Context) -> App {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        let mut app = App {
            catalog,
            asset_root: root,
            prefs: Preferences::default(),
            runtime: None,
            renderer: None,
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
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            preview_next_frame: Instant::now(),
            preview_direction: Direction::Right,
            animation_name: String::new(),
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
        let mut prefs: Preferences =
            serde_json::from_str(r#"{"fps":30,"selected":"pepe"}"#).unwrap();
        assert_eq!(prefs.fps, 30);
        assert!(!prefs.show_in_taskbar);
        assert_eq!(prefs.auto_taskbar_height, cfg!(windows));
        prefs.show_in_taskbar = true;
        prefs.auto_taskbar_height = false;
        prefs.taskbar_height = 120;
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(restored.show_in_taskbar);
        assert_eq!(restored.bottom_height(), Some(120));
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
                        if let egui::Shape::Mesh(mesh) = &shape.shape {
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

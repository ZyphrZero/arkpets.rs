//! Verifies rendering effects on three real, paused Spine models with Metal.

#[cfg(all(target_os = "macos", not(test)))]
mod application {
    #![allow(dead_code)]
    include!("../../src/ui/mod.rs");

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindow};
    use std::{cell::Cell, rc::Rc};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Stage {
        Baseline,
        OutlineHq,
        OutlineLq,
        ShadowHq,
        ShadowLq,
        Combined,
        QuarterOpacity,
        ControlledOutline,
        MipmapsOff,
        MipmapsOn,
    }

    impl Stage {
        const ALL: [Self; 10] = [
            Self::Baseline,
            Self::OutlineHq,
            Self::OutlineLq,
            Self::ShadowHq,
            Self::ShadowLq,
            Self::Combined,
            Self::QuarterOpacity,
            Self::ControlledOutline,
            Self::MipmapsOff,
            Self::MipmapsOn,
        ];

        fn name(self) -> &'static str {
            match self {
                Self::Baseline => "baseline",
                Self::OutlineHq => "outline-hq",
                Self::OutlineLq => "outline-lq",
                Self::ShadowHq => "shadow-hq",
                Self::ShadowLq => "shadow-lq",
                Self::Combined => "combined",
                Self::QuarterOpacity => "combined-quarter-opacity",
                Self::ControlledOutline => "controlled-outline",
                Self::MipmapsOff => "quarter-size-mipmaps-off",
                Self::MipmapsOn => "quarter-size-mipmaps-on",
            }
        }

        fn configure(self, state: &mut App) {
            state.prefs.automatic = self != Self::ControlledOutline;
            let config = &mut state.prefs.config;
            config.property_transition_duration = 0.0;
            config.opacity_normal = if self == Self::QuarterOpacity {
                0.25
            } else {
                1.0
            };
            config.display_scale = if matches!(self, Self::MipmapsOff | Self::MipmapsOn) {
                // Retina otherwise leaves Friston near its source texel density.
                // This also exercises the footprint of 0.5 scale at 1x system DPI.
                0.25
            } else {
                1.0
            };
            let rendering = &mut config.rendering;
            rendering.outline.width = 3.0;
            rendering.outline.color = [0, 255, 0, 255];
            rendering.outline.controlled_color = [255, 128, 0, 255];
            rendering.outline.condition = if matches!(
                self,
                Self::OutlineHq | Self::OutlineLq | Self::Combined | Self::QuarterOpacity
            ) {
                OutlineCondition::Always
            } else {
                OutlineCondition::Never
            };
            rendering.outline.controlled_condition = if self == Self::ControlledOutline {
                OutlineCondition::Always
            } else {
                OutlineCondition::Never
            };
            rendering.shadow_opacity = if matches!(
                self,
                Self::ShadowHq | Self::ShadowLq | Self::Combined | Self::QuarterOpacity
            ) {
                0.7
            } else {
                0.0
            };
            rendering.high_quality = !matches!(self, Self::OutlineLq | Self::ShadowLq);
            rendering.mipmaps = self != Self::MipmapsOff;
            let runtime = state.runtime.as_mut().unwrap();
            runtime
                .apply(PetCommand::Configure(config.clone()))
                .unwrap();
            runtime
                .apply(PetCommand::SetAutomatic(state.prefs.automatic))
                .unwrap();
            runtime
                .apply(PetCommand::SetDirection(Direction::Right))
                .unwrap();
            state.set_paused(true);
        }
    }

    struct Verification {
        app: NativeApp,
        model: usize,
        stage: usize,
        started: Instant,
        requested: bool,
        picture: Option<Arc<egui::ColorImage>>,
        pictures: Vec<Arc<egui::ColorImage>>,
        pose: Vec<Vec<[f32; 2]>>,
        window_number: Option<isize>,
        directory: PathBuf,
        finished: Rc<Cell<bool>>,
        saved: Rc<Cell<bool>>,
    }

    impl Verification {
        fn window(&self) -> objc2::rc::Retained<NSWindow> {
            let windows =
                NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).windows();
            (0..windows.count())
                .map(|index| windows.objectAtIndex(index))
                .find(|window| window.title().to_string() == "ArkPets - 桌宠")
                .expect("native desktop pet window")
        }

        fn changed(before: &egui::ColorImage, after: &egui::ColorImage) -> usize {
            assert_eq!(
                before.size, after.size,
                "effect changed screenshot dimensions"
            );
            before
                .pixels
                .iter()
                .zip(&after.pixels)
                .filter(|(a, b)| a != b)
                .count()
        }

        fn exterior_pixels(&self, picture: &egui::ColorImage, color: [u8; 3]) -> usize {
            let baseline = &self.pictures[0];
            assert_eq!(baseline.size, picture.size);
            baseline
                .pixels
                .iter()
                .zip(&picture.pixels)
                .filter(|(before, after)| {
                    if before.a() != 0 || after.a() < 16 {
                        return false;
                    }
                    after.to_srgba_unmultiplied()[..3]
                        .iter()
                        .zip(color)
                        .all(|(&actual, expected)| actual.abs_diff(expected) <= 3)
                })
                .count()
        }

        fn verify_picture(&mut self, picture: &egui::ColorImage) -> String {
            let stage = Stage::ALL[self.stage];
            assert!(
                picture.pixels.iter().any(|pixel| pixel.a() > 0),
                "empty {stage:?} GPU image"
            );
            // Transparent corners catch accidental full-window fills and smeared FBO edges.
            for x in [0, 1, picture.width() - 2, picture.width() - 1] {
                for y in [0, 1, picture.height() - 2, picture.height() - 1] {
                    assert_eq!(
                        picture[(x, y)].a(),
                        0,
                        "{stage:?}: background corner is opaque"
                    );
                }
            }
            assert!(
                picture.pixels.iter().filter(|pixel| pixel.a() == 0).count()
                    > picture.pixels.len() / 20,
                "{stage:?}: surrounding transparent background disappeared"
            );
            let window = self.window();
            if let Some(number) = self.window_number {
                assert_eq!(
                    window.windowNumber(),
                    number,
                    "effect recreated the native window"
                );
            } else {
                self.window_number = Some(window.windowNumber());
            }
            let mut state = self.app.state.lock().unwrap();
            assert!(state.error.is_none(), "{:?}", state.error);
            assert!(state.paused());
            assert_eq!(state.prefs.automatic, stage != Stage::ControlledOutline);
            assert_eq!(
                state
                    .runtime
                    .as_ref()
                    .unwrap()
                    .character()
                    .animation()
                    .direction,
                Direction::Right
            );
            let mut meshes = Vec::new();
            let spine = &mut state.renderer.as_mut().unwrap().spine;
            assert!(spine.version.starts_with("3.8"));
            spine.meshes(&mut meshes);
            let pose: Vec<_> = meshes.into_iter().map(|mesh| mesh.positions).collect();
            assert!(!pose.is_empty());
            if stage == Stage::Baseline {
                self.pose = pose;
            } else {
                assert_eq!(pose, self.pose, "effect advanced the paused skeleton");
            }
            let model = state.prefs.selected.clone();
            drop(state);
            match stage {
                Stage::Baseline | Stage::MipmapsOff => {}
                Stage::OutlineHq | Stage::OutlineLq | Stage::Combined => {
                    let count = self.exterior_pixels(picture, [0, 255, 0]);
                    assert!(
                        count > 20,
                        "{model}/{stage:?}: no green exterior outline ({count} pixels)"
                    );
                    if stage == Stage::OutlineLq {
                        assert!(
                            Self::changed(&self.pictures[1], picture) > 20,
                            "HQ and LQ outline kernels rendered identically"
                        );
                    }
                }
                Stage::ShadowHq | Stage::ShadowLq => {
                    let count = self.exterior_pixels(picture, [0, 0, 0]);
                    assert!(
                        count > 20,
                        "{model}/{stage:?}: no black exterior shadow ({count} pixels)"
                    );
                    if stage == Stage::ShadowLq {
                        assert!(
                            Self::changed(&self.pictures[3], picture) > 20,
                            "HQ and LQ shadow kernels rendered identically"
                        );
                    }
                }
                Stage::QuarterOpacity => {
                    let combined = &self.pictures[5];
                    assert_eq!(combined.size, picture.size);
                    let mismatches = combined
                        .pixels
                        .iter()
                        .zip(&picture.pixels)
                        .filter(|(before, after)| {
                            before
                                .to_array()
                                .iter()
                                .zip(after.to_array())
                                .any(|(&a, b)| (a as f32 * 0.25 - b as f32).abs() > 2.0)
                        })
                        .count();
                    assert_eq!(mismatches, 0, "{model}: opacity must scale composed model, outline and shadow exactly once");
                }
                Stage::ControlledOutline => {
                    let count = self.exterior_pixels(picture, [255, 128, 0]);
                    assert!(
                        count > 20,
                        "{model}: manual control did not select orange outline ({count} pixels)"
                    );
                    assert_eq!(
                        self.exterior_pixels(picture, [0, 255, 0]),
                        0,
                        "manual mode retained normal outline color"
                    );
                }
                Stage::MipmapsOn => {
                    let changed = Self::changed(&self.pictures[8], picture);
                    assert!(changed > 20, "{model}: MipMap toggle did not change reduced atlas sampling ({changed} pixels)");
                }
            }
            eprintln!(
                "PASS: {model}/{}: {}x{}, paused real-model Metal screenshot",
                stage.name(),
                picture.width(),
                picture.height()
            );
            model
        }

        fn save_pictures(&self, model: &str) {
            for (stage, picture) in Stage::ALL.iter().zip(&self.pictures) {
                let bytes: Vec<_> = picture
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                    .collect();
                image::save_buffer(
                    self.directory.join(format!("{model}-{}.png", stage.name())),
                    &bytes,
                    picture.width() as u32,
                    picture.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }

    impl eframe::App for Verification {
        fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.app.logic(ctx, frame);
            if self.finished.get() {
                return;
            }
            {
                let state = self.app.state.lock().unwrap();
                assert!(
                    state.error.is_none(),
                    "effects application error: {:?}",
                    state.error
                );
                assert!(state.desktop_open, "effects desktop was closed");
                assert!(self.started.elapsed() < Duration::from_secs(8),
                    "model {}/{} screenshot timed out: requested={}, image={}, root_frames={}, desktop_frames={}, bounds={:?}",
                    self.model, Stage::ALL[self.stage].name(), self.requested, self.picture.is_some(),
                    ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT), ctx.cumulative_frame_nr_for(desktop_id()),
                    state.renderer.as_ref().map(|renderer| renderer.bounds));
            }
            if let Some(picture) = self.picture.take() {
                let model = self.verify_picture(&picture);
                self.pictures.push(picture);
                let model_complete = self.stage + 1 == Stage::ALL.len();
                if model_complete {
                    self.save_pictures(&model);
                    if self.model + 1 == self.app.state.lock().unwrap().catalog.models.len() {
                        self.finished.set(true);
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Close,
                        );
                        return;
                    }
                    self.model += 1;
                    self.stage = 0;
                    self.pictures.clear();
                    self.pose.clear();
                } else {
                    self.stage += 1;
                }
                let mut state = self.app.state.lock().unwrap();
                Stage::ALL[self.stage].configure(&mut state);
                if model_complete {
                    state.select(self.model, ctx);
                }
                self.started = Instant::now();
                self.requested = false;
                ctx.request_repaint();
                ctx.request_repaint_of(desktop_id());
            }
            let capture_at = self.started + Duration::from_millis(700);
            if Instant::now() >= capture_at && !self.requested {
                ctx.send_viewport_cmd_to(
                    desktop_id(),
                    egui::ViewportCommand::Screenshot(Default::default()),
                );
                ctx.request_repaint_of(desktop_id());
                self.requested = true;
            }
            request_repaint_at(
                ctx,
                if self.requested {
                    self.started + Duration::from_secs(8)
                } else {
                    capture_at
                },
            );
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.app.ui(ui, frame);
        }

        fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
            for event in &input.events {
                if let egui::Event::Screenshot {
                    viewport_id, image, ..
                } = event
                {
                    assert_eq!(*viewport_id, desktop_id());
                    assert!(self.picture.is_none(), "duplicate GPU screenshot");
                    self.picture = Some(image.clone());
                }
            }
        }

        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            self.app.save(storage);
            self.saved.set(true);
        }
        fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
            self.app.clear_color(visuals)
        }
    }

    pub fn verify() -> eframe::Result {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos-effects");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.ron");
        let finished = Rc::new(Cell::new(false));
        let observed_finished = finished.clone();
        let saved = Rc::new(Cell::new(false));
        let observed_saved = saved.clone();
        eframe::run_native(
            "ArkPets effects verification",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_inner_size([1180.0, 780.0])
                    .with_transparent(true),
                event_loop_builder: Some(Box::new(|builder| {
                    use egui_winit::winit::platform::macos::{
                        ActivationPolicy, EventLoopBuilderExtMacOS as _,
                    };
                    builder.with_activation_policy(ActivationPolicy::Accessory);
                })),
                persist_window: false,
                persistence_path: Some(preferences.clone()),
                ..Default::default()
            },
            Box::new(move |cc| {
                assert_eq!(
                    cc.wgpu_render_state
                        .as_ref()
                        .unwrap()
                        .adapter
                        .get_info()
                        .backend,
                    eframe::wgpu::Backend::Metal
                );
                let mut state = App::new(cc, RendererBackend::default());
                state.prefs = Preferences::default();
                assert_eq!(
                    state.catalog.models.len(),
                    3,
                    "this regression requires the three repository models"
                );
                Stage::Baseline.configure(&mut state);
                state.select(0, &cc.egui_ctx);
                state.desktop_open = true;
                state.page = 1;
                shell_visibility::initialize_application()?;
                let ctx = cc.egui_ctx.clone();
                let menu_bar = menu_bar::MenuBar::new(move || ctx.request_repaint())?;
                Ok(Box::new(Verification {
                    app: NativeApp {
                        state: Arc::new(Mutex::new(state)),
                        menu_bar,
                        dock_visible: None,
                    },
                    model: 0,
                    stage: 0,
                    started: Instant::now(),
                    requested: false,
                    picture: None,
                    pictures: Vec::new(),
                    pose: Vec::new(),
                    window_number: None,
                    directory,
                    finished: observed_finished,
                    saved: observed_saved,
                }))
            }),
        )?;
        assert!(finished.get(), "effects verification interrupted");
        assert!(
            saved.get() && preferences.exists(),
            "normal exit did not save isolated preferences"
        );
        Ok(())
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify().unwrap();
    #[cfg(not(target_os = "macos"))]
    panic!("This native effects verification requires macOS");
}

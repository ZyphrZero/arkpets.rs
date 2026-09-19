//! Verifies transparent mode on a real, paused Metal desktop pet.

#[cfg(all(target_os = "macos", not(test)))]
mod application {
    #![allow(dead_code)]
    include!("../../src/ui/mod.rs");

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindow};
    use std::{cell::Cell, rc::Rc};

    const STAGES: [&str; 4] = ["normal", "transparent", "restored", "instant-transparent"];

    struct Verification {
        app: NativeApp,
        model: usize,
        stage: usize,
        started: Instant,
        requested: bool,
        picture: Option<Arc<egui::ColorImage>>,
        baseline_alpha: u64,
        baseline_picture: Option<egui::ColorImage>,
        window_number: Option<isize>,
        position: Option<PetVec2>,
        directory: PathBuf,
        finished: Rc<Cell<bool>>,
    }

    impl Verification {
        fn window(&self) -> objc2::rc::Retained<NSWindow> {
            let windows =
                NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).windows();
            (0..windows.count())
                .map(|i| windows.objectAtIndex(i))
                .find(|window| window.title().to_string() == "ArkPets - 桌宠")
                .expect("native desktop pet window")
        }

        fn verify_picture(&mut self, picture: &egui::ColorImage) {
            let alpha: u64 = picture.pixels.iter().map(|p| u64::from(p.a())).sum();
            assert!(alpha > 0, "empty desktop image");
            let window = self.window();
            assert_eq!(window.ignoresMouseEvents(), self.stage % 2 == 1);
            if let Some(number) = self.window_number {
                assert_eq!(
                    window.windowNumber(),
                    number,
                    "transparent mode recreated the window"
                );
            } else {
                self.window_number = Some(window.windowNumber());
            }
            if self.stage == 0 {
                self.baseline_alpha = alpha;
                self.baseline_picture = Some(picture.clone());
            } else {
                let ratio = alpha as f64 / self.baseline_alpha as f64;
                if self.stage % 2 == 1 {
                    assert!(
                        (ratio - 0.25).abs() < 0.01,
                        "whole-pet alpha ratio {ratio}, expected 0.25"
                    );
                    let baseline = self.baseline_picture.as_ref().unwrap();
                    assert_eq!(baseline.size, picture.size);
                    let mismatches = baseline
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
                    assert_eq!(mismatches, 0, "overlapping slots must retain the composed color and receive opacity exactly once");
                } else {
                    assert!((ratio - 1.0).abs() < 0.01, "restored alpha ratio {ratio}");
                }
            }
            let state = self.app.state.lock().unwrap();
            assert!(state.error.is_none(), "{:?}", state.error);
            assert!(state.paused());
            let position = state.desktop_motion.as_ref().unwrap().body.position;
            if let Some(before) = self.position {
                assert_eq!(position, before, "opacity changed paused pet position");
            }
            self.position = Some(position);
            assert!(
                !state
                    .desktop_opacity
                    .as_ref()
                    .unwrap()
                    .sample(Instant::now())
                    .1
            );
            let model = state.prefs.selected.clone();
            drop(state);
            let rgba: Vec<u8> = picture
                .pixels
                .iter()
                .flat_map(|p| p.to_srgba_unmultiplied())
                .collect();
            image::save_buffer(
                self.directory
                    .join(format!("{model}-{}.png", STAGES[self.stage])),
                &rgba,
                picture.width() as u32,
                picture.height() as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            println!(
                "PASS: {model}/{}: alpha={alpha}, native passthrough={}, same paused window",
                STAGES[self.stage],
                window.ignoresMouseEvents()
            );
        }
    }

    impl eframe::App for Verification {
        fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.app.logic(ctx, frame);
            if self.finished.get() {
                return;
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(8),
                "opacity screenshot timed out"
            );
            if let Some(picture) = self.picture.take() {
                self.verify_picture(&picture);
                let last_model =
                    self.model + 1 == self.app.state.lock().unwrap().catalog.models.len();
                if self.stage == STAGES.len() - 1 && last_model {
                    self.finished.set(true);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                let mut state = self.app.state.lock().unwrap();
                if self.stage == STAGES.len() - 1 {
                    self.model += 1;
                    self.stage = 0;
                    state.select(self.model, ctx);
                    self.position = None;
                    self.baseline_picture = None;
                } else {
                    self.stage += 1;
                }
                state.prefs.click_through = self.stage % 2 == 1;
                state.prefs.config.property_transition_duration =
                    if self.stage == 3 { 0.0 } else { 0.6 };
                let config = state.prefs.config.clone();
                state
                    .runtime
                    .as_mut()
                    .unwrap()
                    .apply(PetCommand::Configure(config))
                    .unwrap();
                self.started = Instant::now();
                self.requested = false;
                ctx.request_repaint();
                ctx.request_repaint_of(desktop_id());
            }
            let deadline = self.started + Duration::from_millis(1500);
            if Instant::now() >= deadline && !self.requested {
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
                    deadline
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
                    self.picture = Some(image.clone());
                }
            }
        }

        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            self.app.save(storage);
        }

        fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
            self.app.clear_color(visuals)
        }
    }

    pub fn verify() -> eframe::Result {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos-opacity");
        std::fs::create_dir_all(&directory).unwrap();
        let finished = Rc::new(Cell::new(false));
        let observed = finished.clone();
        eframe::run_native(
            "ArkPets opacity verification",
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
                persistence_path: Some(directory.join("preferences.ron")),
                ..Default::default()
            },
            Box::new(move |cc| {
                let mut state = App::new(cc, RendererBackend::default());
                state.prefs = Preferences::default();
                state.prefs.automatic = false;
                state.prefs.config.opacity_dim = 0.25;
                state.prefs.config.rendering.outline.condition = OutlineCondition::Never;
                state.prefs.config.rendering.outline.controlled_condition = OutlineCondition::Never;
                state.prefs.config.property_transition_duration = 0.6;
                state.select(0, &cc.egui_ctx);
                state.set_paused(true);
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
                    baseline_alpha: 0,
                    baseline_picture: None,
                    window_number: None,
                    position: None,
                    directory,
                    finished: observed,
                }))
            }),
        )?;
        assert!(finished.get(), "opacity verification interrupted");
        Ok(())
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify().unwrap();
    #[cfg(not(target_os = "macos"))]
    panic!("This native opacity verification requires macOS");
}

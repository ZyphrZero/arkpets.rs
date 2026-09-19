//! Verifies paused property transitions with real Metal rendering and native windows.

#[cfg(all(target_os = "macos", not(test)))]
mod application {
    #![allow(dead_code)]
    include!("../../src/ui/mod.rs");

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindow};
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug, Clone, Copy)]
    enum Phase {
        Warmup,
        CaptureRight,
        TurnMidpoint,
        CaptureMidpoint,
        TurnComplete,
        CaptureLeft,
        MoveMidpoint,
        MoveComplete,
        InstantMove,
        IdleSettle,
        Idle,
    }

    struct Verification {
        app: NativeApp,
        phase: Phase,
        deadline: Instant,
        transition_started: Instant,
        picture: Option<Arc<egui::ColorImage>>,
        pictures: Vec<(&'static str, Arc<egui::ColorImage>)>,
        pose: Vec<Vec<[f32; 2]>>,
        window_number: Option<isize>,
        initial_native: [f64; 2],
        initial_logical: Pos2,
        target_native_x: f64,
        target_logical_x: f32,
        idle_frames: [u64; 2],
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

        fn pose(state: &mut App) -> Vec<Vec<[f32; 2]>> {
            let mut meshes = Vec::new();
            state.renderer.as_mut().unwrap().spine.meshes(&mut meshes);
            assert!(!meshes.is_empty(), "real Spine pose is empty");
            meshes.into_iter().map(|mesh| mesh.positions).collect()
        }

        fn verify_paused(&self) {
            let mut state = self.app.state.lock().unwrap();
            assert!(state.error.is_none(), "{:?}", state.error);
            assert!(state.paused(), "property transition resumed the simulation");
            assert!(!state.prefs.automatic);
            assert_eq!(
                Self::pose(&mut state),
                self.pose,
                "paused skeleton advanced"
            );
            let window = self.window();
            assert_eq!(Some(window.windowNumber()), self.window_number);
            assert!(window.isVisible(), "desktop window became hidden");
        }

        fn verify_native_position(&self, expected_x: f64) {
            let window = self.window();
            let origin = window.frame().origin;
            let tolerance = 2.0 / window.backingScaleFactor();
            assert!(
                (origin.x - expected_x).abs() <= tolerance,
                "{:?}: native x={}, expected {expected_x} ± {tolerance}",
                self.phase,
                origin.x
            );
            assert!(
                (origin.y - self.initial_native[1]).abs() <= tolerance,
                "horizontal movement changed native y: {} -> {}",
                self.initial_native[1],
                origin.y
            );
        }

        fn request_picture(&mut self, ctx: &egui::Context, phase: Phase) {
            assert!(self.picture.is_none(), "unconsumed GPU screenshot");
            self.phase = phase;
            self.deadline = Instant::now() + Duration::from_secs(5);
            ctx.send_viewport_cmd_to(
                desktop_id(),
                egui::ViewportCommand::Screenshot(Default::default()),
            );
            ctx.request_repaint_of(desktop_id());
        }

        fn alpha_width(picture: &egui::ColorImage) -> usize {
            let mut minimum = picture.width();
            let mut maximum = 0;
            for (index, pixel) in picture.pixels.iter().enumerate() {
                if pixel.a() > 16 {
                    let x = index % picture.width();
                    minimum = minimum.min(x);
                    maximum = maximum.max(x);
                }
            }
            if minimum <= maximum {
                maximum - minimum + 1
            } else {
                0
            }
        }

        fn verify_flip(&self, picture: &egui::ColorImage) {
            let baseline = &self.pictures[0].1;
            assert_eq!(baseline.size, picture.size);
            let changed = baseline
                .pixels
                .iter()
                .zip(&picture.pixels)
                .filter(|(before, after)| before != after)
                .count();
            assert!(
                changed > 100,
                "facing changed without changing rendered pixels"
            );
            let distance = |offset: Option<isize>| -> u64 {
                baseline
                    .pixels
                    .iter()
                    .enumerate()
                    .filter_map(|(index, before)| {
                        let x = index % baseline.width();
                        let y = index / baseline.width();
                        let x = if let Some(offset) = offset {
                            baseline.width() as isize - 1 - x as isize + offset
                        } else {
                            x as isize
                        };
                        if !(0..baseline.width() as isize).contains(&x) {
                            return None;
                        }
                        let after = picture[(x as usize, y)];
                        Some(
                            before
                                .to_array()
                                .iter()
                                .zip(after.to_array())
                                .map(|(&a, b)| u64::from(a.abs_diff(b)))
                                .sum::<u64>(),
                        )
                    })
                    .sum()
            };
            // Rasterization may place the reflection axis between adjacent physical pixels.
            let mirrored = (-2..=2).map(|offset| distance(Some(offset))).min().unwrap();
            let unchanged = distance(None);
            assert!(
                mirrored * 5 < unchanged,
                "final GPU image did not reflect the paused pose: mirror error={mirrored}, unchanged error={unchanged}"
            );
            assert!(
                Self::alpha_width(baseline).abs_diff(Self::alpha_width(picture)) <= 4,
                "finished flip did not restore the full model width"
            );
            eprintln!(
                "PASS: paused GPU facing transition, {changed} changed pixels, reflected pose"
            );
        }

        fn set_position(&mut self, ctx: &egui::Context, logical_x: f32) {
            let mut state = self.app.state.lock().unwrap();
            let motion = state.desktop_motion.as_mut().unwrap();
            let target = Pos2::new(logical_x, motion.screen_position().y);
            assert!(
                motion
                    .constrain_display_position(target, motion.size)
                    .distance(target)
                    < 0.01,
                "test needs 100 logical points of horizontal room on the current screen"
            );
            motion.body.position = motion.local_cursor(target);
            self.target_logical_x = logical_x;
            self.target_native_x = self.initial_native[0]
                + f64::from((logical_x - self.initial_logical.x) * ctx.zoom_factor());
            self.transition_started = Instant::now();
            // One wakeup starts the real application's transition; its own scheduler
            // must supply all subsequent frames until the target is reached.
            ctx.request_repaint_of(desktop_id());
        }

        fn verify_logical_target(&self) {
            let state = self.app.state.lock().unwrap();
            let position = state
                .desktop_motion
                .as_ref()
                .unwrap()
                .last_window_position
                .unwrap();
            assert!(
                (position.x - self.target_logical_x).abs() <= 1.0,
                "displayed logical x={} did not reach {}",
                position.x,
                self.target_logical_x
            );
        }

        fn save_pictures(&self) {
            // PNG encoding runs after the timed checks so it cannot delay a midpoint.
            for (name, picture) in &self.pictures {
                let bytes: Vec<_> = picture
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                    .collect();
                image::save_buffer(
                    self.directory.join(format!("pepe-{name}.png")),
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
            let now = Instant::now();
            let waiting_for_picture = matches!(
                self.phase,
                Phase::CaptureRight | Phase::CaptureMidpoint | Phase::CaptureLeft
            );
            if waiting_for_picture {
                assert!(
                    now < self.deadline,
                    "{:?}: GPU screenshot timed out",
                    self.phase
                );
                if self.picture.is_none() {
                    request_repaint_at(ctx, self.deadline);
                    return;
                }
            } else if now < self.deadline {
                request_repaint_at(ctx, self.deadline);
                return;
            }
            match self.phase {
                Phase::Warmup => self.request_picture(ctx, Phase::CaptureRight),
                Phase::CaptureRight => {
                    let picture = self.picture.take().unwrap();
                    assert!(
                        Self::alpha_width(&picture) > 20,
                        "initial GPU image is empty"
                    );
                    self.pictures.push(("right", picture));
                    let window = self.window();
                    self.window_number = Some(window.windowNumber());
                    self.initial_native = [window.frame().origin.x, window.frame().origin.y];
                    let mut state = self.app.state.lock().unwrap();
                    self.initial_logical = state
                        .desktop_motion
                        .as_ref()
                        .unwrap()
                        .last_window_position
                        .expect("first native desktop frame finished");
                    self.pose = Self::pose(&mut state);
                    state
                        .runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::SetDirection(Direction::Left))
                        .unwrap();
                    self.transition_started = Instant::now();
                    self.deadline = self.transition_started + Duration::from_millis(250);
                    self.phase = Phase::TurnMidpoint;
                    ctx.request_repaint_of(desktop_id());
                }
                Phase::TurnMidpoint => {
                    self.verify_paused();
                    self.verify_native_position(self.initial_native[0]);
                    assert!(
                        self.transition_started.elapsed() < Duration::from_millis(500),
                        "missed facing midpoint observation"
                    );
                    self.request_picture(ctx, Phase::CaptureMidpoint);
                }
                Phase::CaptureMidpoint => {
                    let picture = self.picture.take().unwrap();
                    let baseline_width = Self::alpha_width(&self.pictures[0].1);
                    let midpoint_width = Self::alpha_width(&picture);
                    assert!(
                        midpoint_width * 5 < baseline_width * 4,
                        "facing snapped instead of interpolating: midpoint width={midpoint_width}, original={baseline_width}"
                    );
                    self.pictures.push(("turning", picture));
                    self.phase = Phase::TurnComplete;
                    self.deadline = self.transition_started + Duration::from_millis(950);
                }
                Phase::TurnComplete => self.request_picture(ctx, Phase::CaptureLeft),
                Phase::CaptureLeft => {
                    self.verify_paused();
                    self.verify_native_position(self.initial_native[0]);
                    let picture = self.picture.take().unwrap();
                    self.verify_flip(&picture);
                    self.pictures.push(("left", picture));
                    self.set_position(ctx, self.initial_logical.x + 100.0);
                    self.phase = Phase::MoveMidpoint;
                    self.deadline = self.transition_started + Duration::from_millis(250);
                }
                Phase::MoveMidpoint => {
                    self.verify_paused();
                    assert!(
                        self.transition_started.elapsed() < Duration::from_millis(550),
                        "missed native position midpoint observation"
                    );
                    let x = self.window().frame().origin.x;
                    assert!(
                        x > self.initial_native[0] + 2.0 && x < self.target_native_x - 2.0,
                        "position must interpolate between native x={} and {}, observed {x}",
                        self.initial_native[0],
                        self.target_native_x
                    );
                    eprintln!("PASS: native position intermediate x={x}");
                    self.phase = Phase::MoveComplete;
                    self.deadline = self.transition_started + Duration::from_millis(950);
                }
                Phase::MoveComplete => {
                    self.verify_paused();
                    self.verify_native_position(self.target_native_x);
                    self.verify_logical_target();
                    eprintln!(
                        "PASS: paused native position transition reached x={}",
                        self.target_native_x
                    );
                    {
                        let mut state = self.app.state.lock().unwrap();
                        assert!(state.prefs.config.allow_drag);
                        state.prefs.config.property_transition_duration = 0.0;
                        state.prefs.config.allow_interact = false;
                        let config = state.prefs.config.clone();
                        let runtime = state.runtime.as_mut().unwrap();
                        let animation = runtime.character().animation().animation.clone();
                        runtime.apply(PetCommand::Configure(config)).unwrap();
                        assert!(!runtime.character_mut().click());
                        assert_eq!(runtime.character().animation().animation, animation);
                        assert!(state.paused());
                        assert!(state.prefs.config.allow_drag);
                    }
                    self.set_position(ctx, self.initial_logical.x);
                    self.phase = Phase::InstantMove;
                    self.deadline = self.transition_started + Duration::from_millis(100);
                }
                Phase::InstantMove => {
                    self.verify_paused();
                    self.verify_native_position(self.target_native_x);
                    self.verify_logical_target();
                    eprintln!("PASS: zero duration moves immediately; disabling interaction preserves pause and drag preference");
                    self.phase = Phase::IdleSettle;
                    self.deadline = now + Duration::from_millis(500);
                }
                Phase::IdleSettle => {
                    self.idle_frames = [
                        ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT),
                        ctx.cumulative_frame_nr_for(desktop_id()),
                    ];
                    self.phase = Phase::Idle;
                    self.deadline = now + Duration::from_millis(2500);
                }
                Phase::Idle => {
                    self.verify_paused();
                    self.verify_native_position(self.target_native_x);
                    let root_frames =
                        ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) - self.idle_frames[0];
                    let desktop_frames =
                        ctx.cumulative_frame_nr_for(desktop_id()) - self.idle_frames[1];
                    eprintln!("Idle observation: launcher={root_frames}, desktop={desktop_frames}");
                    assert!(
                        root_frames <= 8,
                        "idle launcher continuously repainted: {root_frames}"
                    );
                    assert!(
                        desktop_frames <= 8,
                        "completed properties continuously repainted: {desktop_frames}"
                    );
                    eprintln!("PASS: settled 2.5 seconds: launcher={root_frames} frames, desktop={desktop_frames} frames");
                    self.save_pictures();
                    self.finished.set(true);
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                    return;
                }
            }
            request_repaint_at(ctx, self.deadline);
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.app.ui(ui, frame);
        }

        fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
            if matches!(self.phase, Phase::Idle) && !input.events.is_empty() {
                let pointers = input
                    .events
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            egui::Event::PointerMoved(_)
                                | egui::Event::PointerGone
                                | egui::Event::MouseMoved(_)
                        )
                    })
                    .count();
                eprintln!(
                    "Idle input: viewport={:?}, events={}, pointer_events={pointers}",
                    input.viewport_id,
                    input.events.len()
                );
            }
            for event in &input.events {
                if let egui::Event::Screenshot {
                    viewport_id, image, ..
                } = event
                {
                    assert_eq!(*viewport_id, desktop_id());
                    assert!(self.picture.is_none(), "unexpected duplicate screenshot");
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
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos-behavior");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.ron");
        let finished = Rc::new(Cell::new(false));
        let observed_finished = finished.clone();
        let saved = Rc::new(Cell::new(false));
        let observed_saved = saved.clone();
        eframe::run_native(
            "ArkPets behavior verification",
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
                    eframe::wgpu::Backend::Metal,
                    "this regression requires the real Metal backend"
                );
                let mut state = App::new(cc, RendererBackend::default());
                state.prefs = Preferences::default();
                state.prefs.automatic = false;
                state.prefs.config.property_transition_duration = 0.6;
                state.prefs.config.easing = EasingFunction::Linear;
                state.prefs.config.rendering.outline.condition = OutlineCondition::Never;
                state.prefs.config.rendering.outline.controlled_condition = OutlineCondition::Never;
                state.select(0, &cc.egui_ctx);
                assert_eq!(state.prefs.selected, "pepe");
                assert!(state
                    .renderer
                    .as_ref()
                    .unwrap()
                    .spine
                    .version
                    .starts_with("3.8"));
                state
                    .runtime
                    .as_mut()
                    .unwrap()
                    .apply(PetCommand::SetDirection(Direction::Right))
                    .unwrap();
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
                    phase: Phase::Warmup,
                    deadline: Instant::now() + Duration::from_secs(1),
                    transition_started: Instant::now(),
                    picture: None,
                    pictures: Vec::new(),
                    pose: Vec::new(),
                    window_number: None,
                    initial_native: [0.0; 2],
                    initial_logical: Pos2::ZERO,
                    target_native_x: 0.0,
                    target_logical_x: 0.0,
                    idle_frames: [0; 2],
                    directory,
                    finished: observed_finished,
                    saved: observed_saved,
                }))
            }),
        )?;
        assert!(finished.get(), "behavior verification interrupted");
        assert!(
            saved.get() && preferences.exists(),
            "normal exit did not save preferences"
        );
        Ok(())
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify().unwrap();
    #[cfg(not(target_os = "macos"))]
    panic!("This native behavior verification requires macOS");
}

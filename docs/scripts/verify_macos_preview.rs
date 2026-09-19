//! Verifies continuous preview rendering without synthetic pointer events.

#[cfg(all(target_os = "macos", not(test)))]
mod application {
    #![allow(dead_code)]
    include!("../../src/ui/mod.rs");

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindow};
    use std::{cell::Cell, rc::Rc};

    const TITLE: &str = "ArkPets preview animation verification";
    const SCENARIOS: [&str; 9] = [
        "desktop-and-preview",
        "minimized",
        "restored",
        "hidden",
        "shown",
        "settings",
        "preview-only",
        "paused",
        "resumed",
    ];

    struct Verification {
        app: NativeApp,
        stage: usize,
        started: Instant,
        frames: usize,
        pictures: Vec<Arc<egui::ColorImage>>,
        directory: PathBuf,
        finished: Rc<Cell<bool>>,
    }

    impl Verification {
        fn animated(&self) -> bool {
            matches!(self.stage, 0 | 2 | 4 | 6 | 8)
        }

        fn window(&self) -> objc2::rc::Retained<NSWindow> {
            let windows =
                NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).windows();
            (0..windows.count())
                .map(|index| windows.objectAtIndex(index))
                .find(|window| window.title().to_string() == TITLE)
                .expect("native launcher window")
        }

        fn verify_stage(&self) {
            let scenario = SCENARIOS[self.stage];
            if self.animated() {
                assert!(
                    self.frames >= 30,
                    "{scenario}: preview stopped without pointer input ({} frames)",
                    self.frames
                );
                assert_eq!(
                    self.pictures.len(),
                    2,
                    "{scenario}: missing GPU screenshots"
                );
                let first = &self.pictures[0];
                let second = &self.pictures[1];
                assert_eq!(first.size, second.size);
                // Retina screenshots contain physical pixels. Crop the preview,
                // excluding the CPU status text, toolbar and side panels.
                let scale = first.width() as f32 / 1180.0;
                let changed = ((100.0 * scale) as usize..(700.0 * scale) as usize)
                    .flat_map(|y| {
                        ((250.0 * scale) as usize..(900.0 * scale) as usize).map(move |x| (x, y))
                    })
                    .filter(|&(x, y)| first[(x, y)] != second[(x, y)])
                    .count();
                assert!(
                    changed > 100,
                    "{scenario}: model pixels did not animate ({changed})"
                );
                // Encode only after sampling; debug PNG compression can block
                // the UI thread long enough to distort the cadence check.
                for (index, image) in self.pictures.iter().enumerate() {
                    let path = self.directory.join(format!("{scenario}-{index}.png"));
                    let bytes: Vec<_> = image
                        .pixels
                        .iter()
                        .flat_map(|pixel| pixel.to_array())
                        .collect();
                    image::save_buffer(
                        path,
                        &bytes,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
                eprintln!(
                    "PASS {scenario}: {} UI frames, {changed} changed preview pixels",
                    self.frames
                );
            } else {
                assert!(
                    self.frames <= 8,
                    "{scenario}: unnecessary continuous preview work ({} frames)",
                    self.frames
                );
                if matches!(self.stage, 1 | 3 | 5) {
                    let state = self.app.state.lock().unwrap();
                    assert!(
                        state.desktop_motion.as_ref().unwrap().last_tick
                            > self.started + Duration::from_secs(1),
                        "{scenario}: desktop animation also stopped"
                    );
                }
                eprintln!("PASS {scenario}: {} UI frames after settling", self.frames);
            }
        }
    }

    impl eframe::App for Verification {
        fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.app.logic(ctx, frame);
            if self.finished.get() {
                return;
            }
            let deadline = self.started + Duration::from_secs(3);
            if Instant::now() < deadline {
                request_repaint_at(ctx, deadline);
                return;
            }
            self.verify_stage();
            if self.stage == SCENARIOS.len() - 1 {
                self.finished.set(true);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            self.stage += 1;
            let window = self.window();
            let mut state = self.app.state.lock().unwrap();
            assert!(state.error.is_none(), "{:?}", state.error);
            match self.stage {
                1 => window.miniaturize(None),
                2 => window.deminiaturize(None),
                3 => window.orderOut(None),
                4 => window.orderFront(None),
                5 => state.page = 1,
                6 => {
                    state.page = 0;
                    state.desktop_open = false;
                }
                7 => state.set_paused(true),
                8 => state.set_paused(false),
                _ => unreachable!(),
            }
            self.frames = 0;
            self.pictures.clear();
            self.started = Instant::now();
            request_repaint_at(ctx, self.started + Duration::from_secs(3));
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.app.ui(ui, frame);
            if self.started.elapsed() < Duration::from_millis(500) {
                return;
            }
            self.frames += 1;
            if !self.animated() {
                return;
            }
            if matches!(self.frames, 5 | 25) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }

        fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
            // GPU readbacks can arrive in either viewport's next input batch.
            for event in &input.events {
                if let egui::Event::Screenshot {
                    viewport_id, image, ..
                } = event
                {
                    assert_eq!(*viewport_id, egui::ViewportId::ROOT);
                    self.pictures.push(image.clone());
                }
            }
        }

        fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
            self.app.clear_color(visuals)
        }
    }

    pub fn verify() -> eframe::Result {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos-preview");
        std::fs::create_dir_all(&directory).unwrap();
        let finished = Rc::new(Cell::new(false));
        let observed = finished.clone();
        eframe::run_native(
            TITLE,
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title(TITLE)
                    .with_transparent(true)
                    .with_inner_size([1180.0, 780.0])
                    .with_resizable(false),
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
                state.prefs.automatic = false;
                state
                    .runtime
                    .as_mut()
                    .unwrap()
                    .apply(PetCommand::SetAutomatic(false))
                    .unwrap();
                state.set_paused(false);
                state.desktop_open = true;
                state.page = 0;
                shell_visibility::initialize_application()?;
                let ctx = cc.egui_ctx.clone();
                let menu_bar = menu_bar::MenuBar::new(move || ctx.request_repaint())?;
                Ok(Box::new(Verification {
                    app: NativeApp {
                        state: Arc::new(Mutex::new(state)),
                        menu_bar,
                        dock_visible: None,
                    },
                    stage: 0,
                    started: Instant::now(),
                    frames: 0,
                    pictures: Vec::new(),
                    directory,
                    finished: observed,
                }))
            }),
        )?;
        assert!(finished.get(), "preview verification interrupted");
        println!("PASS: preview animates without pointer input; hidden/minimized/settings/paused viewports stay idle and resume correctly");
        Ok(())
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify().unwrap();
    #[cfg(not(target_os = "macos"))]
    panic!("This native preview verification requires macOS");
}

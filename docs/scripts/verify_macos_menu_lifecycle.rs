//! Exercises the production Metal UI and menu request handler with a minimized/hidden launcher.

#[cfg(all(target_os = "macos", not(test)))]
mod application {
    // The normal CLI entry point is unused by this native verification wrapper.
    #![allow(dead_code)]
    include!("../../src/ui/mod.rs");

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindow};
    use std::{cell::Cell, rc::Rc};

    const TITLE: &str = "ArkPets menu lifecycle verification";

    struct Verification {
        app: NativeApp,
        stage: usize,
        deadline: Instant,
        ui_ready: bool,
        finished: Rc<Cell<bool>>,
        toolbar_steps: usize,
        toolbar_tick: Option<Instant>,
        icon: objc2::rc::Retained<objc2_app_kit::NSImage>,
    }

    fn window(title: &str) -> objc2::rc::Retained<NSWindow> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let windows = app.windows();
        (0..windows.count())
            .map(|index| windows.objectAtIndex(index))
            .find(|window| window.title().to_string() == title)
            .unwrap_or_else(|| panic!("missing native window {title}"))
    }

    impl eframe::App for Verification {
        fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.app.logic(ctx, frame);
            if self.finished.get() {
                return;
            }
            if !self.ui_ready || Instant::now() < self.deadline {
                ctx.request_repaint_after(self.deadline.saturating_duration_since(Instant::now()));
                return;
            }
            let launcher = window(TITLE);
            let pet = window("ArkPets - 桌宠");
            let mut state = self.app.state.lock().unwrap();
            assert!(state.error.is_none(), "UI error: {:?}", state.error);
            let native_app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            assert_eq!(
                native_app.applicationIconImage().as_deref(),
                Some(&*self.icon)
            );
            assert_eq!(
                native_app.activationPolicy(),
                if state.prefs.show_in_taskbar {
                    objc2_app_kit::NSApplicationActivationPolicy::Regular
                } else {
                    objc2_app_kit::NSApplicationActivationPolicy::Accessory
                }
            );
            if self.toolbar_steps < 10 {
                eprintln!(
                    "toolbar step {}: pet_visible={} desktop_open={}",
                    self.toolbar_steps,
                    pet.isVisible(),
                    state.desktop_open
                );
                let step = self.toolbar_steps % 5;
                if step > 0 && step.is_multiple_of(2) {
                    assert!(pet.isVisible(), "toolbar reopen left the pet hidden");
                    let tick = state.desktop_motion.as_ref().unwrap().last_tick;
                    assert!(
                        tick > self.toolbar_tick.unwrap(),
                        "desktop animation stopped after toolbar reopen"
                    );
                }
                state.desktop_open = step % 2 == 1;
                if self.toolbar_steps == 4 {
                    state.prefs.show_in_taskbar = true;
                } else if self.toolbar_steps == 9 {
                    state.prefs.show_in_taskbar = false;
                }
                self.toolbar_tick = Some(Instant::now());
                self.toolbar_steps += 1;
                self.deadline = Instant::now() + Duration::from_secs(1);
                ctx.request_repaint_after(Duration::from_secs(1));
                return;
            }
            let mut requests = menu_bar::Requests::default();
            match self.stage {
                0 => {
                    assert!(!pet.isVisible());
                    state.set_paused(true);
                    launcher.miniaturize(None);
                }
                1 => {
                    assert!(launcher.isMiniaturized());
                    requests.desktop_open = Some(true);
                }
                2 => {
                    assert!(pet.isVisible(), "menu start failed with minimized launcher");
                    assert!(
                        launcher.isMiniaturized(),
                        "menu start restored the launcher"
                    );
                    assert!(state.desktop_open && state.paused());
                    requests.desktop_open = Some(false);
                }
                3 => {
                    assert!(!pet.isVisible(), "menu stop left pet visible");
                    assert!(launcher.isMiniaturized());
                    assert!(!state.desktop_open && state.desktop_motion.is_none());
                    launcher.deminiaturize(None);
                }
                4 => {
                    assert!(!launcher.isMiniaturized());
                    launcher.orderOut(None);
                }
                5 => {
                    assert!(!launcher.isVisible());
                    requests.desktop_open = Some(true);
                }
                6 => {
                    assert!(pet.isVisible(), "menu reopen failed with hidden launcher");
                    assert!(
                        !launcher.isVisible(),
                        "menu start revealed a hidden launcher"
                    );
                    requests.desktop_open = Some(false);
                }
                7 => {
                    assert!(!pet.isVisible());
                    assert!(
                        !launcher.isVisible(),
                        "menu stop revealed a hidden launcher"
                    );
                    requests.show_launcher = true;
                }
                8 => {
                    assert!(launcher.isVisible() && !launcher.isMiniaturized());
                    assert!(!pet.isVisible());
                    assert!(state.paused());
                    requests.quit = true;
                    self.finished.set(true);
                }
                _ => unreachable!(),
            }
            state.handle_menu_requests(ctx, requests);
            self.stage += 1;
            self.deadline = Instant::now() + Duration::from_secs(1);
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.app.ui(ui, frame);
            self.ui_ready = true;
        }

        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            self.app.save(storage);
        }

        fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
            self.app.clear_color(visuals)
        }
    }

    pub fn verify() -> eframe::Result {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos-menu-lifecycle");
        std::fs::create_dir_all(&directory).unwrap();
        let finished = Rc::new(Cell::new(false));
        let observed = finished.clone();
        eframe::run_native(
            TITLE,
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title(TITLE)
                    .with_transparent(true)
                    .with_inner_size([1180.0, 780.0]),
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
                let state = Arc::new(Mutex::new(App::new(cc, RendererBackend::default())));
                state.lock().unwrap().prefs.show_in_taskbar = false;
                shell_visibility::initialize_application()?;
                let icon = NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
                    .applicationIconImage()
                    .expect("ArkPets Dock icon");
                let ctx = cc.egui_ctx.clone();
                let menu_bar = menu_bar::MenuBar::new(move || ctx.request_repaint())?;
                Ok(Box::new(Verification {
                    app: NativeApp {
                        state,
                        menu_bar,
                        dock_visible: None,
                    },
                    stage: 0,
                    deadline: Instant::now() + Duration::from_secs(1),
                    ui_ready: false,
                    finished: observed,
                    toolbar_steps: 0,
                    toolbar_tick: None,
                    icon,
                }))
            }),
        )?;
        assert!(finished.get(), "native lifecycle verification interrupted");
        println!("PASS: actual Metal UI starts/stops/reopens pet with launcher minimized or hidden; only Show Launcher restores it");
        Ok(())
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify().unwrap();
    #[cfg(not(target_os = "macos"))]
    panic!("This native menu lifecycle verification requires macOS");
}

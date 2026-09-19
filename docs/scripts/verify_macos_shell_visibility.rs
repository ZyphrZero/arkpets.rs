//! Run with `cargo run --locked --example verify-macos-shell-visibility` on a macOS desktop.
//! Uses real AppKit windows and the production policy implementation.

#[cfg(target_os = "macos")]
#[path = "../../src/ui/shell_visibility/macos.rs"]
mod shell_visibility;

#[cfg(target_os = "macos")]
mod verification {
    use super::shell_visibility;
    use egui_winit::winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::{Window, WindowId, WindowLevel},
    };
    use objc2::{rc::Retained, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWindow};
    use std::{
        process::{Child, Command},
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Verification {
        windows: Vec<Window>,
        stage: usize,
        deadline: Option<Instant>,
        background_process: Option<Child>,
    }

    fn native_window(app: &NSApplication, title: &str) -> Retained<NSWindow> {
        let windows = app.windows();
        (0..windows.count())
            .map(|index| windows.objectAtIndex(index))
            .find(|window| window.title().to_string() == title)
            .expect("native verification window exists")
    }

    fn create_pet(event_loop: &ActiveEventLoop) -> Window {
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("ArkPets - 桌宠")
                    .with_visible(false)
                    .with_inner_size(egui_winit::winit::dpi::LogicalSize::new(420.0, 240.0))
                    .with_decorations(false)
                    .with_window_level(WindowLevel::AlwaysOnTop),
            )
            .unwrap()
    }

    impl ApplicationHandler for Verification {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if !self.windows.is_empty() {
                return;
            }
            if std::env::args().any(|argument| argument == "--focus-helper") {
                self.windows.push(
                    event_loop
                        .create_window(
                            Window::default_attributes().with_title("ArkPets focus check helper"),
                        )
                        .unwrap(),
                );
                event_loop.set_control_flow(ControlFlow::Wait);
                return;
            }
            shell_visibility::initialize_application().unwrap();
            for (title, visible) in [
                ("ArkPets visibility check - hidden", false),
                ("ArkPets visibility check - minimized", true),
                ("ArkPets visibility check - launcher", true),
            ] {
                let attributes = Window::default_attributes()
                    .with_title(title)
                    .with_visible(visible)
                    .with_inner_size(egui_winit::winit::dpi::LogicalSize::new(420.0, 240.0));
                self.windows
                    .push(event_loop.create_window(attributes).unwrap());
            }
            self.windows.push(create_pet(event_loop));
            self.windows[1].set_minimized(true);
            self.windows[2].focus_window();
            self.deadline = Some(Instant::now() + Duration::from_secs(2));
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if event == WindowEvent::CloseRequested {
                event_loop.exit();
                panic!("verification interrupted by closing a window");
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            let Some(deadline) = self.deadline else {
                return;
            };
            if Instant::now() < deadline {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                return;
            }
            let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            let launcher = native_window(&app, "ArkPets visibility check - launcher");
            let visible_pets = {
                let windows = app.windows();
                (0..windows.count())
                    .map(|index| windows.objectAtIndex(index))
                    .filter(|window| {
                        window.title().to_string() == "ArkPets - 桌宠" && window.isVisible()
                    })
                    .count()
            };
            let hidden = native_window(&app, "ArkPets visibility check - hidden");
            let minimized = native_window(&app, "ArkPets visibility check - minimized");
            println!(
                "stage {}: active={}, launcher_key={}, visible_pets={visible_pets}",
                self.stage,
                app.isActive(),
                launcher.isKeyWindow()
            );
            assert!(
                launcher.isVisible(),
                "launcher disappeared at stage {}",
                self.stage
            );
            assert!(
                !hidden.isVisible(),
                "hidden window was unexpectedly restored"
            );
            assert!(
                minimized.isMiniaturized(),
                "minimized window was unexpectedly restored"
            );
            match self.stage {
                0 => {
                    assert!(app.isActive(), "verification must start in the foreground");
                    assert_eq!(visible_pets, 0);
                    shell_visibility::set_visible(false).unwrap();
                    shell_visibility::set_application_visible(false).unwrap();
                    // Matches eframe showing the pet after its first rendered frame.
                    self.windows[3].set_visible(true);
                }
                1 | 3 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Accessory
                    );
                    assert_eq!(visible_pets, 1);
                    assert!(app.isActive(), "policy transition deactivated the launcher");
                    assert!(
                        launcher.isKeyWindow(),
                        "pet took launcher focus during the transition"
                    );
                    shell_visibility::set_visible(true).unwrap();
                    shell_visibility::set_application_visible(true).unwrap();
                }
                2 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Regular
                    );
                    assert!(app.isActive());
                    assert!(launcher.isKeyWindow());
                    shell_visibility::set_visible(false).unwrap();
                    shell_visibility::set_application_visible(false).unwrap();
                }
                4 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Regular
                    );
                    shell_visibility::set_visible(false).unwrap();
                    shell_visibility::set_application_visible(false).unwrap();
                }
                5 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Accessory
                    );
                    assert!(launcher.isKeyWindow());
                    shell_visibility::set_application_visible(true).unwrap();
                    // eframe drops the viewport after the app restores Dock policy.
                    drop(self.windows.pop().unwrap());
                }
                6 => {
                    assert!(
                        visible_pets == 0,
                        "closed pet was restored by a pending policy callback"
                    );
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Regular
                    );
                    assert!(launcher.isKeyWindow());
                    self.windows.push(create_pet(event_loop));
                    shell_visibility::set_visible(false).unwrap();
                    shell_visibility::set_application_visible(false).unwrap();
                    self.windows[3].set_visible(true);
                }
                7 => {
                    assert_eq!(
                        visible_pets, 1,
                        "reopening must create exactly one visible pet"
                    );
                    assert!(app.isActive());
                    assert!(launcher.isKeyWindow());
                    // Also close with focus on the pet, as in its context menu.
                    native_window(&app, "ArkPets - 桌宠").makeKeyAndOrderFront(None);
                    shell_visibility::set_application_visible(true).unwrap();
                    drop(self.windows.pop().unwrap());
                }
                8 => {
                    assert_eq!(visible_pets, 0, "second close restored an obsolete window");
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Regular
                    );
                    assert!(launcher.isKeyWindow());
                    self.windows.push(create_pet(event_loop));
                    shell_visibility::set_visible(true).unwrap();
                    shell_visibility::set_application_visible(true).unwrap();
                    self.windows[3].set_visible(true);
                }
                9 => {
                    assert_eq!(visible_pets, 1);
                    // Closing with Dock enabled must also remove the native window.
                    drop(self.windows.pop().unwrap());
                }
                10 => {
                    assert_eq!(visible_pets, 0);
                    self.background_process = Some(
                        Command::new(std::env::current_exe().unwrap())
                            .arg("--focus-helper")
                            .spawn()
                            .expect("start isolated foreground app"),
                    );
                }
                11 => {
                    assert!(!app.isActive());
                    shell_visibility::set_application_visible(false).unwrap();
                }
                12 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Accessory
                    );
                    assert!(!app.isActive(), "background policy change stole focus");
                    shell_visibility::set_application_visible(true).unwrap();
                }
                13 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Regular
                    );
                    assert!(!app.isActive(), "background policy restoration stole focus");
                    assert_eq!(visible_pets, 0);
                    let mut child = self.background_process.take().unwrap();
                    child.kill().expect("stop focus helper");
                    child.wait().expect("reap focus helper");
                    self.deadline = None;
                    println!("PASS: launcher visibility/focus, Dock policy toggles, hidden/minimized windows, native pet destruction/reopen and background transitions");
                    event_loop.exit();
                    return;
                }
                _ => unreachable!(),
            }
            self.stage += 1;
            let deadline = Instant::now() + Duration::from_secs(1);
            self.deadline = Some(deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }

    impl Drop for Verification {
        fn drop(&mut self) {
            if let Some(child) = &mut self.background_process {
                child
                    .kill()
                    .expect("stop focus helper after interrupted verification");
                child.wait().expect("reap focus helper");
            }
        }
    }

    pub fn run() {
        EventLoop::new()
            .unwrap()
            .run_app(&mut Verification::default())
            .unwrap();
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    verification::run();
    #[cfg(not(target_os = "macos"))]
    panic!("This native window verification requires macOS");
}

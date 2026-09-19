//! Native menu actions, Dock independence, and status-item cleanup.
//! Run on a macOS desktop with `cargo run --locked --example verify-macos-menu-bar`.

#[cfg(target_os = "macos")]
#[path = "../../src/ui/shell_visibility/macos.rs"]
mod shell_visibility;

#[cfg(target_os = "macos")]
mod native {
    // Include the production module here so the verifier can dispatch actual menu
    // actions and inspect native objects without adding a diagnostic product API.
    include!("../../src/ui/menu_bar.rs");

    use egui_winit::winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::{Window, WindowId},
    };
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSStatusBarButton};
    use std::{
        rc::Rc,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Verification {
        windows: Vec<Window>,
        bar: Option<MenuBar>,
        button: Option<Retained<NSStatusBarButton>>,
        wakes: Rc<Cell<usize>>,
        stage: usize,
        deadline: Option<Instant>,
        interrupted: bool,
    }

    impl ApplicationHandler for Verification {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if !self.windows.is_empty() {
                return;
            }
            super::shell_visibility::initialize_application().unwrap();
            for title in ["ArkPets menu bar verification", "ArkPets - 桌宠"] {
                self.windows.push(
                    event_loop
                        .create_window(Window::default_attributes().with_title(title))
                        .unwrap(),
                );
            }
            let wakes = self.wakes.clone();
            let bar = MenuBar::new(move || wakes.set(wakes.get() + 1)).unwrap();
            self.button = bar.item.button(MainThreadMarker::new().unwrap());
            self.bar = Some(bar);
            self.deadline = Some(Instant::now() + Duration::from_secs(1));
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if event == WindowEvent::CloseRequested {
                self.interrupted = true;
                self.deadline = None;
                event_loop.exit();
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
            let main_thread = MainThreadMarker::new().unwrap();
            let app = NSApplication::sharedApplication(main_thread);
            let button = self.button.as_ref().unwrap();
            match self.stage {
                0 => {
                    let bar = self.bar.as_ref().unwrap();
                    assert!(bar.item.isVisible());
                    assert!(button.window().is_some_and(|window| window.isVisible()));
                    assert!(
                        button.title().to_string().is_empty(),
                        "menu bar must show an icon without title text"
                    );
                    assert_eq!(button.imagePosition(), NSCellImagePosition::ImageOnly);
                    assert!(button.image().expect("menu bar icon").isTemplate());
                    assert_eq!(button.toolTip().unwrap().to_string(), "ArkPets 桌宠");
                    bar.update(false, false);
                    assert!(!bar.desktop_item.isEnabled());
                    bar.update(false, true);
                    assert!(bar.desktop_item.isEnabled());
                    let menu = bar.item.menu(main_thread).unwrap();
                    menu.performActionForItemAtIndex(0);
                    assert_eq!(
                        bar.take_requests(),
                        Requests {
                            show_launcher: true,
                            ..Default::default()
                        }
                    );
                    menu.performActionForItemAtIndex(1);
                    assert_eq!(bar.take_requests().desktop_open, Some(true));
                    bar.update(true, true);
                    assert_eq!(bar.desktop_item.title().to_string(), "收起桌宠");
                    assert_eq!(self.wakes.get(), 2);
                    super::shell_visibility::set_visible(false).unwrap();
                    super::shell_visibility::set_application_visible(false).unwrap();
                }
                1 => {
                    assert_eq!(
                        app.activationPolicy(),
                        NSApplicationActivationPolicy::Accessory
                    );
                    let bar = self.bar.as_ref().unwrap();
                    assert!(
                        bar.item.isVisible(),
                        "hiding Dock must retain the menu bar entry"
                    );
                    assert!(button.window().is_some_and(|window| window.isVisible()));
                    let menu = bar.item.menu(main_thread).unwrap();
                    menu.performActionForItemAtIndex(1);
                    assert_eq!(bar.take_requests().desktop_open, Some(false));
                    bar.update(false, true);
                    assert_eq!(bar.desktop_item.title().to_string(), "启动桌宠");
                    // Repeated toggles before the next frame retain the final intent.
                    menu.performActionForItemAtIndex(1);
                    menu.performActionForItemAtIndex(1);
                    assert_eq!(bar.take_requests().desktop_open, Some(false));
                    menu.performActionForItemAtIndex(3);
                    assert!(bar.take_requests().quit);
                    assert_eq!(bar.take_requests(), Requests::default());
                    assert_eq!(self.wakes.get(), 6);
                    super::shell_visibility::set_application_visible(true).unwrap();
                    self.bar.take();
                }
                2 => {
                    assert!(
                        button.window().is_none_or(|window| !window.isVisible()),
                        "removed menu bar button is still on screen"
                    );
                    assert_eq!(
                        Rc::strong_count(&self.wakes),
                        1,
                        "menu target retained its wake callback after shutdown"
                    );
                    self.deadline = None;
                    println!("PASS: native menu entry/actions, Dock independence, toggle synchronization and cleanup");
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

    pub fn verify() {
        let mut verification = Verification::default();
        EventLoop::new()
            .unwrap()
            .run_app(&mut verification)
            .unwrap();
        assert!(
            !verification.interrupted,
            "verification interrupted by closing a window"
        );
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    native::verify();
    #[cfg(not(target_os = "macos"))]
    panic!("This native menu bar verification requires macOS");
}

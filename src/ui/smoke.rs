//! Opt-in native GPU capture sequence, enabled only in development builds.
use super::*;

const SHOTS: [&str; 9] = [
    "01-pepe",
    "02-pepe-motion",
    "03-whitw2",
    "04-friston",
    "05-settings-light",
    "06-settings-dark",
    "07-desktop",
    "08-desktop-floor",
    "09-desktop-walk",
];

pub struct Capture {
    directory: PathBuf,
    stage: usize,
    frames: usize,
    pending: bool,
    stage_time: Instant,
    desktop_positions: Vec<Pos2>,
}

impl Capture {
    pub fn from_env() -> Option<Self> {
        std::env::var_os("ARKPETS_CAPTURE_DIR").map(|path| Self {
            directory: path.into(),
            stage: 0,
            frames: 0,
            pending: false,
            stage_time: Instant::now(),
            desktop_positions: Vec::new(),
        })
    }

    pub fn receive(&mut self, ctx: &egui::Context, frame_ms: f32) {
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            if let egui::Event::Screenshot {
                viewport_id, image, ..
            } = event
            {
                let expected = if self.stage < 6 {
                    egui::ViewportId::ROOT
                } else {
                    egui::ViewportId::from_hash_of("desktop-pet")
                };
                if !self.pending || self.stage >= SHOTS.len() || viewport_id != expected {
                    continue;
                }
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                std::fs::create_dir_all(&self.directory).expect("create capture directory");
                let path = self.directory.join(format!("{}.png", SHOTS[self.stage]));
                image::save_buffer(
                    &path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .expect("save native GPU screenshot");
                eprintln!(
                    "Capture {}: {}x{}, CPU {:.2} ms",
                    path.display(),
                    image.width(),
                    image.height(),
                    frame_ms
                );
                self.stage += 1;
                self.frames = 0;
                self.pending = false;
                ctx.request_repaint_of(egui::ViewportId::ROOT);
                if self.stage == SHOTS.len() {
                    self.verify();
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                    return;
                }
            }
        }
    }

    pub fn update(&mut self, app: &mut App, ctx: &egui::Context) {
        self.receive(ctx, app.frame_ms);
        if self.stage >= SHOTS.len() {
            return;
        }
        if self.frames == 0 {
            match self.stage {
                0 => {
                    app.select(0, ctx);
                    app.prefs.dark = false;
                    app.theme(ctx);
                    app.prefs.automatic = false;
                    if let Some(runtime) = &mut app.runtime {
                        runtime
                            .apply(arkpets::PetCommand::SetAutomatic(false))
                            .expect("disable automatic animation for GPU capture");
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        1180.0, 780.0,
                    )));
                }
                2 => app.select(1, ctx),
                3 => app.select(2, ctx),
                4 => {
                    app.page = 1;
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        940.0, 640.0,
                    )));
                }
                5 => {
                    app.prefs.dark = true;
                    app.theme(ctx);
                }
                6 => {
                    app.desktop_open = true;
                    app.prefs.automatic = false;
                    if let Some(runtime) = &mut app.runtime {
                        runtime
                            .apply(arkpets::PetCommand::SetAutomatic(false))
                            .expect("disable automatic desktop animation for GPU capture");
                        assert!(runtime.character_mut().play_animation("Relax"));
                    }
                    app.sync_animation(true);
                }
                8 => {
                    if let Some(character) =
                        app.runtime.as_mut().map(arkpets::PetRuntime::character_mut)
                    {
                        assert!(character.play_animation("Move"));
                        character.set_direction(Direction::Right);
                    }
                    app.sync_animation(true);
                }
                _ => {}
            }
            if self.stage >= 6 {
                self.stage_time = Instant::now();
            }
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        self.frames += 1;
        if self.frames >= 40 && !self.pending && self.stage < 6 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.pending = true;
        }
        if self.stage < 6 {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    pub fn desktop(&mut self, ctx: &egui::Context, frame_ms: f32) {
        self.receive(ctx, frame_ms);
        let wait = match self.stage {
            6 => 0.2,
            7 => 2.0,
            _ => 1.0,
        };
        if self.stage >= 6
            && self.stage < SHOTS.len()
            && self.frames > 0
            && self.stage_time.elapsed().as_secs_f32() >= wait
            && !self.pending
        {
            let position = ctx.input(|input| {
                input
                    .viewport()
                    .outer_rect
                    .expect("desktop window geometry")
                    .min
            });
            eprintln!("Desktop OS position: {position:?}");
            self.desktop_positions.push(position);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.pending = true;
        }
    }

    fn verify(&self) {
        let load = |name: &str| {
            image::open(self.directory.join(format!("{name}.png")))
                .unwrap()
                .into_rgba8()
        };
        let first = load(SHOTS[0]);
        let second = load(SHOTS[1]);
        let changed = (100..700)
            .flat_map(|y| (250..900).map(move |x| (x, y)))
            .filter(|&(x, y)| first.get_pixel(x, y) != second.get_pixel(x, y))
            .count();
        assert!(
            changed > 100,
            "animation did not visibly change: {changed} pixels"
        );
        let light = load(SHOTS[4]);
        let dark = load(SHOTS[5]);
        assert!(
            light.get_pixel(900, 350)[0] > 220,
            "light settings background is not filled"
        );
        assert!(
            dark.get_pixel(900, 350)[0] < 70,
            "dark settings background is incorrect"
        );
        let desktop = load(SHOTS[6]);
        let opaque = desktop.pixels().filter(|p| p[3] > 128).count();
        let clear = desktop.pixels().filter(|p| p[3] == 0).count();
        assert!(
            opaque > 100 && clear > 100,
            "desktop must have a visible model and transparent surroundings"
        );
        assert!(
            self.desktop_positions[1].y > self.desktop_positions[0].y + 20.0,
            "native desktop window did not fall"
        );
        assert!(
            self.desktop_positions[2].x > self.desktop_positions[1].x + 5.0,
            "native desktop window did not walk"
        );
        eprintln!("GPU verification passed: {changed} animated pixels, {opaque} model pixels, {clear} transparent pixels");
    }
}

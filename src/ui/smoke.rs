//! Opt-in native GPU capture sequence, enabled only in development builds.
use super::*;

const SHOTS: [&str; 48] = [
    "01-pepe",
    "02-pepe-motion",
    "03-whitw2",
    "04-friston",
    "05-settings-light",
    "06-settings-dark",
    "07-desktop",
    "08-desktop-floor",
    "09-desktop-walk",
    "10-whitw2-attack",
    "11-whitw2-attack-motion",
    "12-whitw2-attack-desktop",
    "13-whitw2-attack-left",
    "14-whitw2-floor-auto",
    "15-whitw2-floor-120",
    "16-whitw2-floor-240",
    "17-pepe-ground-auto",
    "18-pepe-ground-120",
    "19-whitw2-ground-auto",
    "20-whitw2-ground-120",
    "21-friston-ground-auto",
    "22-friston-ground-120",
    "23-battle-ground-auto",
    "24-battle-ground-120",
    "25-amiya-desktop",
    "26-amiya-motion",
    "27-amiya-battle-idle",
    "28-amiya-battle-motion",
    "29-surtr-desktop",
    "30-surtr-motion",
    "31-surtr-battle-idle",
    "32-surtr-battle-motion",
    "33-texas2-desktop",
    "34-texas2-motion",
    "35-texas2-battle-idle",
    "36-texas2-battle-motion",
    "37-logos-desktop",
    "38-logos-motion",
    "39-logos-battle-idle",
    "40-logos-battle-motion",
    "41-texas-forms-library",
    "42-texas-selected-battle",
    "43-texas-selected-base-walk",
    "44-texas-selected-battle-rest",
    "45-texas-fixed-base",
    "46-texas-fixed-battle-loop",
    "47-texas-selected-base-move",
    "48-texas-fixed-battle-resume",
];

pub struct Capture {
    directory: PathBuf,
    stage: usize,
    frames: usize,
    pending: bool,
    stage_time: Instant,
    desktop_positions: Vec<Pos2>,
    ground_pixel: Option<f32>,
    form_anchor: Option<Pos2>,
}

impl Capture {
    fn desktop_stage(&self) -> bool {
        matches!(self.stage, 6..=8 | 11..=39 | 41..=47)
    }

    pub fn from_env() -> Option<Self> {
        std::env::var_os("ARKPETS_CAPTURE_DIR").map(|path| Self {
            directory: path.into(),
            stage: 0,
            frames: 0,
            pending: false,
            stage_time: Instant::now(),
            desktop_positions: Vec::new(),
            ground_pixel: None,
            form_anchor: None,
        })
    }

    pub fn receive(&mut self, ctx: &egui::Context, frame_ms: f32) {
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            if let egui::Event::Screenshot {
                viewport_id, image, ..
            } = event
            {
                let expected = if self.desktop_stage() {
                    egui::ViewportId::from_hash_of("desktop-pet")
                } else {
                    egui::ViewportId::ROOT
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
                if let Some(ground) = self.ground_pixel.take() {
                    let visible_bottom = image
                        .pixels
                        .iter()
                        .enumerate()
                        .filter(|(_, pixel)| pixel.a() > 128)
                        .map(|(index, _)| index / image.width() + 1)
                        .max()
                        .expect("standing model has visible pixels")
                        as f32;
                    let gap = ground - visible_bottom;
                    assert!(
                        gap.abs() <= 3.0,
                        "{}: visible standing bottom differs from floor by {gap} physical pixels",
                        SHOTS[self.stage]
                    );
                    eprintln!(
                        "{} visible ground PASS: gap={gap:.2} physical pixels",
                        SHOTS[self.stage]
                    );
                }
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
                9 => {
                    app.desktop_open = false;
                    app.desktop_motion = None;
                    app.page = 0;
                    let index = app
                        .catalog
                        .models
                        .iter()
                        .position(|model| model.id == "whitw2-battle")
                        .expect("bundled battle model");
                    app.select(index, ctx);
                    assert!(app.error.is_none(), "{:?}", app.error);
                    let runtime = app.runtime.as_mut().unwrap();
                    runtime.apply(PetCommand::SetAutomatic(false)).unwrap();
                    runtime
                        .apply(PetCommand::PlayAnimation("Attack".into()))
                        .unwrap();
                    app.sync_animation(true);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        1180.0, 780.0,
                    )));
                }
                11 => {
                    app.desktop_open = true;
                    app.prefs.config.property_transition_duration = 0.0;
                    app.prefs.config.rendering.outline.controlled_condition =
                        OutlineCondition::Always;
                    let runtime = app.runtime.as_mut().unwrap();
                    runtime
                        .apply(PetCommand::Configure(app.prefs.config.clone()))
                        .unwrap();
                    runtime
                        .apply(PetCommand::SetDirection(Direction::Right))
                        .unwrap();
                    runtime.apply(PetCommand::SetPaused(true)).unwrap();
                    let renderer = app.renderer.as_mut().unwrap();
                    renderer.spine.play_with_mix("Attack", true, 0.0).unwrap();
                    // Freeze the widest attack pose for both directions/backends.
                    let duration = renderer.spine.animation_duration("Attack").unwrap();
                    let mut meshes = Vec::new();
                    let mut widest = (0, 0.0_f32);
                    renderer.update(0.0, 0.0).unwrap();
                    for frame in 0..=(duration * 60.0).ceil() as usize {
                        renderer.spine.meshes(&mut meshes);
                        let mut bounds = Rect::NOTHING;
                        for mesh in &meshes {
                            for &[x, y] in &mesh.positions {
                                bounds.extend_with(Pos2::new(x, y));
                            }
                        }
                        if bounds.width() > widest.1 {
                            widest = (frame, bounds.width());
                        }
                        renderer.update(1.0 / 60.0, 0.0).unwrap();
                    }
                    renderer.spine.play_with_mix("Attack", true, 0.0).unwrap();
                    renderer.update(0.0, 0.0).unwrap();
                    for _ in 0..widest.0 {
                        renderer.update(1.0 / 60.0, 0.0).unwrap();
                    }
                }
                12 => {
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::SetDirection(Direction::Left))
                        .unwrap();
                }
                13..=15 => {
                    app.prefs.multi_monitors = false;
                    app.prefs.auto_taskbar_height = self.stage == 13;
                    app.prefs.taskbar_height = if self.stage == 14 { 120 } else { 240 };
                    // Preserve the same body position while changing the floor.
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::SetPaused(false))
                        .unwrap();
                    if let Some(motion) = &mut app.desktop_motion {
                        motion.next_work_area_update = Instant::now();
                    }
                    ctx.request_repaint_of(desktop_id());
                }
                16..=23 => {
                    let id = ["pepe", "whitw2", "frston-3", "whitw2-battle"][(self.stage - 16) / 2];
                    app.prefs.auto_taskbar_height = self.stage.is_multiple_of(2);
                    app.prefs.taskbar_height = 120;
                    app.prefs.initial_position = [0.5, 1.0];
                    app.prefs.config.rendering.shadow_opacity = 0.0;
                    app.prefs.config.rendering.outline.controlled_condition =
                        OutlineCondition::Never;
                    app.desktop_motion = None;
                    let index = app
                        .catalog
                        .models
                        .iter()
                        .position(|model| model.id == id)
                        .unwrap();
                    app.select(index, ctx);
                    assert!(app.error.is_none(), "{:?}", app.error);
                    app.desktop_open = true;
                    // Deploy a fixed real standing pose at the floor. Free fall
                    // and moving/attacking animations are exercised above.
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::SetPaused(true))
                        .unwrap();
                    ctx.request_repaint_of(desktop_id());
                }
                24..=39 if self.stage.is_multiple_of(2) => {
                    let id = [
                        "amiya",
                        "amiya-battle",
                        "surtr",
                        "surtr-battle",
                        "texas2",
                        "texas2-battle",
                        "logos",
                        "logos-battle",
                    ][(self.stage - 24) / 2];
                    let index = app
                        .catalog
                        .models
                        .iter()
                        .position(|model| model.id == id)
                        .unwrap();
                    app.desktop_motion = None;
                    app.select(index, ctx);
                    assert!(app.error.is_none(), "{id}: {:?}", app.error);
                    let battle = id.ends_with("-battle");
                    app.select_form(if battle {
                        arkpets::model::ModelForm::Battle
                    } else {
                        arkpets::model::ModelForm::Base
                    })
                    .unwrap();
                    let runtime = app.runtime.as_mut().unwrap();
                    runtime.apply(PetCommand::SetPaused(false)).unwrap();
                    runtime.apply(PetCommand::SetAutomatic(false)).unwrap();
                    app.sync_animation(true);
                    ctx.request_repaint_of(desktop_id());
                }
                40 => {
                    let index = app
                        .catalog
                        .models
                        .iter()
                        .position(|model| model.id == "texas2-battle")
                        .unwrap();
                    app.desktop_motion = None;
                    app.select(index, ctx);
                    app.select_form(arkpets::model::ModelForm::Battle).unwrap();
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::PlayAnimation("Idle".into()))
                        .unwrap();
                    app.set_paused(true);
                    app.desktop_open = true;
                    app.page = 0;
                    app.sync_animation(true);
                }
                42 | 43 => {
                    app.select_form(if self.stage == 42 {
                        arkpets::model::ModelForm::Base
                    } else {
                        arkpets::model::ModelForm::Battle
                    })
                    .unwrap();
                    let name = if self.stage == 42 { "Move" } else { "Idle" };
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::PlayAnimation(name.into()))
                        .unwrap();
                    app.sync_animation(false);
                }
                44 => app.select_form(arkpets::model::ModelForm::Base).unwrap(),
                45 => {
                    app.select_form(arkpets::model::ModelForm::Battle).unwrap();
                    app.prefs.config.animation_mix_duration = 0.0;
                    let runtime = app.runtime.as_mut().unwrap();
                    runtime
                        .apply(PetCommand::Configure(app.prefs.config.clone()))
                        .unwrap();
                    runtime
                        .apply(PetCommand::PlayAnimation("Skill_2_Loop".into()))
                        .unwrap();
                    app.sync_animation(true);
                }
                46 | 47 => {
                    app.select_form(if self.stage == 46 {
                        arkpets::model::ModelForm::Base
                    } else {
                        arkpets::model::ModelForm::Battle
                    })
                    .unwrap();
                    let name = if self.stage == 46 {
                        "Move"
                    } else {
                        "Skill_2_Loop"
                    };
                    app.runtime
                        .as_mut()
                        .unwrap()
                        .apply(PetCommand::PlayAnimation(name.into()))
                        .unwrap();
                    app.sync_animation(false);
                }
                _ => {}
            }
            if self.desktop_stage() {
                self.stage_time = Instant::now();
            }
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        self.frames += 1;
        if self.frames >= 40 && !self.pending && !self.desktop_stage() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.pending = true;
        }
        if !self.desktop_stage() {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    pub fn desktop(&mut self, ctx: &egui::Context, frame_ms: f32, body: Rect, floor: f32) {
        self.receive(ctx, frame_ms);
        let wait = match self.stage {
            6 => 0.2,
            7 => 2.0,
            13..=15 => 3.0,
            24..=39 => 0.37,
            _ => 1.0,
        };
        if self.desktop_stage()
            && self.stage < SHOTS.len()
            && self.frames > 0
            && self.stage_time.elapsed().as_secs_f32() >= wait
            && !self.pending
        {
            if matches!(self.stage, 41..=47) {
                let anchor = ctx.input(|input| input.viewport().inner_rect.unwrap().min)
                    + Vec2::new(body.center().x, body.bottom());
                if let Some(original) = self.form_anchor {
                    assert!(
                        anchor.distance(original) <= 1.5 / ctx.pixels_per_point(),
                        "form feet moved: {original:?} -> {anchor:?}"
                    );
                } else {
                    self.form_anchor = Some(anchor);
                }
                assert!((anchor.y - floor).abs() <= 1.5 / ctx.pixels_per_point());
                eprintln!("{} form anchor PASS: {anchor:?}", SHOTS[self.stage]);
            }
            if matches!(self.stage, 13..=23) {
                let actual_bottom =
                    ctx.input(|input| input.viewport().inner_rect.unwrap().min.y) + body.bottom();
                assert!((actual_bottom - floor).abs() <= 1.5 / ctx.pixels_per_point(), "{}: displayed body bottom {actual_bottom} differs from work-area floor {floor}", SHOTS[self.stage]);
                eprintln!("{} floor PASS: displayed body bottom={actual_bottom:.2}, work-area floor={floor:.2}", SHOTS[self.stage]);
            }
            if matches!(self.stage, 16..=23) {
                self.ground_pixel = Some(body.bottom() * ctx.pixels_per_point());
            }
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
        let attack = load(SHOTS[9]);
        let attack_motion = load(SHOTS[10]);
        let attack_changed = (100..700)
            .flat_map(|y| (250..900).map(move |x| (x, y)))
            .filter(|&(x, y)| attack.get_pixel(x, y) != attack_motion.get_pixel(x, y))
            .count();
        assert!(
            attack_changed > 100,
            "Attack did not visibly change: {attack_changed} pixels"
        );
        for name in &SHOTS[11..=12] {
            let attack_desktop = load(name);
            assert!(attack_desktop.pixels().filter(|p| p[3] > 128).count() > 100);
            assert!(attack_desktop.pixels().filter(|p| p[3] == 0).count() > 100);
            let (width, height) = attack_desktop.dimensions();
            assert!(
                attack_desktop.enumerate_pixels().all(|(x, y, pixel)| {
                    (x >= 2 && x + 2 < width && y >= 2 && y + 2 < height) || pixel[3] == 0
                }),
                "{name}: visible weapon/outline touches the window edge"
            );
        }
        eprintln!("Battle GPU verification passed: {attack_changed} animated attack pixels");
        for pair in SHOTS[24..40].as_chunks::<2>().0 {
            let first = load(pair[0]);
            let next = load(pair[1]);
            assert_eq!(
                first.dimensions(),
                next.dimensions(),
                "{} changed canvas size",
                pair[0]
            );
            assert!(
                first.pixels().filter(|pixel| pixel[3] > 128).count() > 100,
                "{} is invisible",
                pair[0]
            );
            let changed = first
                .pixels()
                .zip(next.pixels())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                changed > 10,
                "{} did not animate: {changed} pixels",
                pair[0]
            );
            eprintln!(
                "New model GPU PASS: {} ({changed} animated pixels)",
                pair[0]
            );
        }
        eprintln!("GPU verification passed: {changed} animated pixels, {opaque} model pixels, {clear} transparent pixels");
    }
}

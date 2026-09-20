use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize)]
pub struct Settings {
    pub model: String,
    pub form_mode: super::forms::Mode,
    pub page: String,
    pub fps: u32,
    pub warmup_seconds: u64,
    pub duration_seconds: u64,
}

#[derive(Clone)]
pub struct Profile {
    settings: Settings,
    directory: PathBuf,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    device: Option<eframe::wgpu::Device>,
    desktop_geometry: Option<[f32; 5]>,
    controls: Option<serde_json::Value>,
    started: Option<Instant>,
    started_unix_ms: u128,
    root: Frames,
    desktop: Frames,
    animation_updates: usize,
    finished: bool,
}

#[derive(Default)]
struct Frames {
    last: Option<Instant>,
    intervals_ms: Vec<f64>,
    work_ms: Vec<f64>,
}

#[derive(Serialize)]
struct Distribution {
    samples: usize,
    mean: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

#[derive(Serialize)]
struct FrameReport {
    callback_rate_hz: Option<f64>,
    callback_interval_ms: Option<Distribution>,
    intervals_above_1_5_frame_budgets: usize,
    update_ms: Option<Distribution>,
}

#[derive(Serialize)]
struct Report<'a> {
    version: u32,
    process_id: u32,
    os: &'static str,
    architecture: &'static str,
    debug_assertions: bool,
    scenario: &'static str,
    metric_scope: &'static str,
    settings: &'a Settings,
    measurement_start_unix_ms: u128,
    measurement_end_unix_ms: u128,
    root: FrameReport,
    desktop: FrameReport,
    animation_updates: usize,
    animation_update_rate_hz: f64,
}

impl Profile {
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(directory) = env::var_os("ARKPETS_PROFILE_DIR") else {
            return Ok(None);
        };
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "Cannot create performance directory {}: {error}",
                directory.display()
            )
        })?;
        let settings = Settings {
            model: env::var("ARKPETS_PROFILE_MODEL").unwrap_or_else(|_| "pepe".into()),
            form_mode: match env::var("ARKPETS_PROFILE_FORM").as_deref() {
                Err(env::VarError::NotPresent) | Ok("auto") => super::forms::Mode::Auto,
                Ok("base") => super::forms::Mode::Base,
                Ok("battle") => super::forms::Mode::Battle,
                _ => return Err("ARKPETS_PROFILE_FORM must be auto, base or battle".into()),
            },
            page: match env::var("ARKPETS_PROFILE_PAGE") {
                Err(env::VarError::NotPresent) => "settings".into(),
                Ok(page) if matches!(page.as_str(), "library" | "settings") => page,
                _ => return Err("ARKPETS_PROFILE_PAGE must be library or settings".into()),
            },
            fps: number("ARKPETS_PROFILE_FPS", 60, 1, 240)? as u32,
            warmup_seconds: number("ARKPETS_PROFILE_WARMUP", 2, 2, 60)?,
            duration_seconds: number("ARKPETS_PROFILE_SECONDS", 8, 8, 120)?,
        };
        Ok(Some(Self {
            settings,
            directory,
            state: Default::default(),
        }))
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn record_device(&self, device: eframe::wgpu::Device) {
        self.state
            .lock()
            .expect("performance recorder lock poisoned")
            .device = Some(device);
    }

    /// Expose the collision area for native interaction checks. Canvas padding
    /// can extend past the floor, so HWND bounds alone cannot verify landing.
    pub fn record_desktop_geometry(
        &self,
        body: eframe::egui::Rect,
        pixels_per_point: f32,
    ) -> Result<(), String> {
        let geometry = [
            body.min.x,
            body.min.y,
            body.width(),
            body.height(),
            pixels_per_point,
        ];
        let mut state = self
            .state
            .lock()
            .expect("performance recorder lock poisoned");
        if state.desktop_geometry == Some(geometry) {
            return Ok(());
        }
        let json = serde_json::json!({
            "body": {"x": body.min.x, "y": body.min.y, "width": body.width(), "height": body.height()},
            "pixels_per_point": pixels_per_point,
        });
        let temporary = self.directory.join("desktop-geometry.tmp");
        fs::write(&temporary, json.to_string())
            .and_then(|()| fs::rename(&temporary, self.directory.join("desktop-geometry.json")))
            .map_err(|error| format!("Cannot write desktop geometry: {error}"))?;
        state.desktop_geometry = Some(geometry);
        Ok(())
    }

    pub fn record_controls(
        &self,
        runtime: &arkpets::PetRuntime,
        walking: Option<arkpets::interaction::WalkControl>,
        active_model: &str,
        form: Option<arkpets::model::ModelForm>,
    ) -> Result<(), String> {
        let value = serde_json::json!({
            "animation": runtime.character().animation().animation,
            "direction": runtime.character().animation().direction,
            "automatic": runtime.automatic(),
            "paused": runtime.paused(),
            "walking": walking.is_some(),
            "fast": walking.is_some_and(|control| control.fast),
            "walk_speed": runtime.character().config().walk_speed,
            "active_model": active_model,
            "form": form,
        });
        let mut state = self
            .state
            .lock()
            .expect("performance recorder lock poisoned");
        if state.controls.as_ref() == Some(&value) {
            return Ok(());
        }
        let temporary = self.directory.join("controls.tmp");
        fs::write(&temporary, value.to_string())
            .and_then(|()| fs::rename(&temporary, self.directory.join("controls.json")))
            .map_err(|error| format!("Cannot write control diagnostic: {error}"))?;
        state.controls = Some(value);
        Ok(())
    }

    pub fn root_update_finished(&self, started: Instant) {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .expect("performance recorder lock poisoned");
        if let Some((begin, end)) = self.window(&state) {
            state.root.record(now, begin, end, Some(started.elapsed()));
        }
    }

    /// Records UI callback completion, before GPU submission and presentation.
    pub fn desktop_callback_finished(&self, animation_advanced: bool) {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .expect("performance recorder lock poisoned");
        if state.started.is_none() {
            state.started = Some(now);
            state.started_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock precedes Unix epoch")
                .as_millis();
        }
        let (begin, end) = self.window(&state).expect("desktop recording has started");
        state.desktop.record(now, begin, end, None);
        if animation_advanced && now >= begin && now <= end {
            state.animation_updates += 1;
        }
    }

    /// Poll from the active viewport; a true result asks the caller to close ROOT.
    pub fn finish_if_ready(&self) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .expect("performance recorder lock poisoned");
        if state.finished {
            return Ok(true);
        }
        let Some((_, end)) = self.window(&state) else {
            return Ok(false);
        };
        if Instant::now() < end {
            return Ok(false);
        }
        if state.desktop.intervals_ms.is_empty() {
            return Err(
                "No desktop callback intervals were captured during the measurement window".into(),
            );
        }
        let begin_unix_ms = state.started_unix_ms + self.settings.warmup_seconds as u128 * 1000;
        let report = Report {
            version: 1,
            process_id: std::process::id(),
            os: env::consts::OS,
            architecture: env::consts::ARCH,
            debug_assertions: cfg!(debug_assertions),
            scenario: match self.settings.page.as_str() {
                "library" => "desktop pet with library page visible; one model at scale 1; Relax animation",
                _ => "desktop pet with settings page visible; one model at scale 1; Relax animation",
            },
            metric_scope: "Callback cadence and CPU animation update rate are not display-present FPS. Update durations are CPU wall time. GPU scan-out is not measured.",
            settings: &self.settings,
            measurement_start_unix_ms: begin_unix_ms,
            measurement_end_unix_ms: begin_unix_ms + self.settings.duration_seconds as u128 * 1000,
            root: state.root.report(self.settings.fps),
            desktop: state.desktop.report(self.settings.fps),
            animation_updates: state.animation_updates,
            animation_update_rate_hz: state.animation_updates as f64 / self.settings.duration_seconds as f64,
        };
        let json = serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("Cannot serialize performance report: {error}"))?;
        let path = self.directory.join("frames.json");
        fs::write(&path, json).map_err(|error| {
            format!(
                "Cannot write performance report {}: {error}",
                path.display()
            )
        })?;
        self.write_gpu_allocations(&state)?;
        state.finished = true;
        eprintln!("Performance report: {}", path.display());
        Ok(true)
    }

    fn write_gpu_allocations(&self, _state: &State) -> Result<(), String> {
        let allocations = _state
            .device
            .as_ref()
            .and_then(|device| device.generate_allocator_report())
            .map(|report| {
                serde_json::json!({
                    "total_allocated_bytes": report.total_allocated_bytes,
                    "total_reserved_bytes": report.total_reserved_bytes,
                    "allocations": report.allocations.iter().map(|allocation| {
                        serde_json::json!({
                            "name": allocation.name,
                            "offset": allocation.offset,
                            "size": allocation.size,
                        })
                    }).collect::<Vec<_>>(),
                    "blocks": report.blocks.iter().map(|block| {
                        serde_json::json!({
                            "size": block.size,
                            "allocation_start": block.allocations.start,
                            "allocation_end": block.allocations.end,
                        })
                    }).collect::<Vec<_>>(),
                })
            });
        let report = serde_json::json!({
            "version": 1,
            "process_id": std::process::id(),
            "metric_scope": "wgpu resource allocator at measurement end; excludes swapchains, descriptor heaps, command allocators, driver memory, and CPU application allocations",
            "supported": allocations.is_some(),
            "renderer_api": if _state.device.is_some() { "wgpu" } else { "OpenGL" },
            "allocator": allocations,
        });
        let json = serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("Cannot serialize GPU allocation report: {error}"))?;
        let path = self.directory.join("gpu-allocations.json");
        fs::write(&path, json).map_err(|error| {
            format!(
                "Cannot write GPU allocation report {}: {error}",
                path.display()
            )
        })
    }

    fn window(&self, state: &State) -> Option<(Instant, Instant)> {
        let begin = state.started? + Duration::from_secs(self.settings.warmup_seconds);
        Some((
            begin,
            begin + Duration::from_secs(self.settings.duration_seconds),
        ))
    }
}

impl Frames {
    fn record(&mut self, now: Instant, begin: Instant, end: Instant, work: Option<Duration>) {
        if now >= begin && now <= end {
            if let Some(previous) = self.last.filter(|previous| *previous >= begin) {
                self.intervals_ms
                    .push(now.duration_since(previous).as_secs_f64() * 1000.0);
            }
            if let Some(work) = work {
                self.work_ms.push(work.as_secs_f64() * 1000.0);
            }
        }
        self.last = Some(now);
    }

    fn report(&self, fps: u32) -> FrameReport {
        let distribution = Distribution::of(&self.intervals_ms);
        FrameReport {
            callback_rate_hz: distribution.as_ref().map(|values| 1000.0 / values.mean),
            callback_interval_ms: distribution,
            intervals_above_1_5_frame_budgets: self
                .intervals_ms
                .iter()
                .filter(|interval| **interval > 1500.0 / fps as f64)
                .count(),
            update_ms: Distribution::of(&self.work_ms),
        }
    }
}

impl Distribution {
    fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable_by(f64::total_cmp);
        // Nearest-rank quantiles keep long-frame outliers visible in small samples.
        let percentile =
            |fraction: f64| sorted[(fraction * sorted.len() as f64).ceil() as usize - 1];
        Some(Self {
            samples: sorted.len(),
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50: percentile(0.5),
            p95: percentile(0.95),
            p99: percentile(0.99),
            max: *sorted.last().unwrap(),
        })
    }
}

fn number(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64, String> {
    match env::var(name) {
        Err(env::VarError::NotPresent) => Ok(default),
        value => value
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (minimum..=maximum).contains(value))
            .ok_or_else(|| format!("{name} must be an integer between {minimum} and {maximum}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rate_uses_callback_intervals_and_excludes_warmup() {
        let begin = Instant::now();
        let end = begin + Duration::from_millis(100);
        let mut frames = Frames::default();
        for milliseconds in [-10, 10, 30, 50, 70, 90, 110] {
            let now = if milliseconds < 0 {
                begin - Duration::from_millis((-milliseconds) as u64)
            } else {
                begin + Duration::from_millis(milliseconds as u64)
            };
            frames.record(now, begin, end, Some(Duration::from_millis(1)));
        }
        let report = frames.report(60);
        assert_eq!(report.callback_rate_hz, Some(50.0));
        assert_eq!(report.callback_interval_ms.unwrap().samples, 4);
        assert_eq!(report.update_ms.unwrap().mean, 1.0);
    }

    #[test]
    fn quantiles_preserve_long_frames() {
        let mut values = vec![16.0; 98];
        values.extend([80.0, 200.0]);
        let report = Distribution::of(&values).unwrap();
        assert_eq!(report.p50, 16.0);
        assert_eq!(report.p95, 16.0);
        assert_eq!(report.p99, 80.0);
        assert_eq!(report.max, 200.0);
        assert!(Distribution::of(&[]).is_none());
    }
}

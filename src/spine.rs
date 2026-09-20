//! Thin ownership wrapper around the pinned official Spine 3.8 runtime.
//!
//! Each instance owns its complete C object graph and can move between threads.
//! Model creation is serialized because Spine 3.8's JSON parser, attachment ID
//! counter, and empty-animation initialization contain mutable process globals.
//! Playback and mesh extraction require exclusive access to an instance.
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    fs,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::Mutex,
};
use thiserror::Error;

static LOAD_LOCK: Mutex<()> = Mutex::new(());

type MeshCallback = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    c_int,
    *const f32,
    *const f32,
    c_int,
    *const u16,
    c_int,
    f32,
    f32,
    f32,
    f32,
);

unsafe extern "C" {
    fn ark_spine_load(
        atlas: *const c_char,
        atlas_len: c_int,
        data: *const u8,
        data_len: c_int,
        json: c_int,
        error: *mut c_char,
        error_len: c_int,
    ) -> *mut c_void;
    fn ark_spine_free(pet: *mut c_void);
    fn ark_spine_animation_count(pet: *mut c_void) -> c_int;
    fn ark_spine_animation_name(pet: *mut c_void, index: c_int) -> *const c_char;
    fn ark_spine_animation_duration(
        pet: *const c_void,
        name: *const c_char,
        duration: *mut f32,
    ) -> c_int;
    fn ark_spine_sample_animation(pet: *mut c_void, name: *const c_char, time: f32) -> c_int;
    fn ark_spine_page(pet: *mut c_void, index: c_int) -> *const c_char;
    fn ark_spine_play(pet: *mut c_void, name: *const c_char, looping: c_int) -> c_int;
    fn ark_spine_play_with_mix(
        pet: *mut c_void,
        name: *const c_char,
        looping: c_int,
        mix: f32,
    ) -> c_int;
    fn ark_spine_update(pet: *mut c_void, delta: f32, mix: f32) -> c_int;
    fn ark_spine_meshes(pet: *mut c_void, context: *mut c_void, emit: MeshCallback);
}

#[derive(Debug, Error)]
pub enum SpineError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Spine: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default)]
pub struct MeshPart {
    pub page: usize,
    pub blend: i32,
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    pub color: [f32; 4],
}

pub struct Spine {
    raw: NonNull<c_void>,
    pub animations: Vec<String>,
    pub pages: Vec<PathBuf>,
    pub version: String,
}

// SAFETY: ArkSpine owns all skeleton, animation, atlas, and clipping allocations;
// it contains no GPU handles or thread-local references. C callbacks run inline
// and never retain Rust borrows. LOAD_LOCK protects Json.ep, VertexAttachment's
// nextID, and AnimationState's initial SP_EMPTY_ANIMATION allocation. The bridge
// never changes global allocator callbacks or Bone.yDown, calls disposeStatics,
// or uses the random vertex effects. play/update/meshes/drop only mutate the
// exclusively owned graph and use the default thread-safe C allocator. NonNull
// keeps Spine !Sync; every mutating runtime operation requires &mut self or ownership.
unsafe impl Send for Spine {}

impl Spine {
    /// Loads trusted, locally installed Spine assets. The upstream C parser is
    /// not a sandbox for hostile or arbitrarily truncated model files.
    pub fn load(skeleton: &Path, atlas: &Path) -> Result<Self, SpineError> {
        let read = |path: &Path| {
            fs::read(path).map_err(|source| SpineError::Io {
                path: path.into(),
                source,
            })
        };
        let mut data = read(skeleton)?;
        let atlas_data = read(atlas)?;
        let json = data.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
        let version = if json {
            let value: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|err| SpineError::Invalid(err.to_string()))?;
            value["skeleton"]["spine"].as_str().unwrap_or("").to_owned()
        } else {
            let mut bytes = data.as_slice();
            binary_string(&mut bytes)?;
            binary_string(&mut bytes)?.to_owned()
        };
        if !version.starts_with("3.8.") {
            return Err(SpineError::Invalid(format!(
                "unsupported skeleton version {version:?}; expected 3.8.x"
            )));
        }
        let data_len = c_int::try_from(data.len())
            .map_err(|_| SpineError::Invalid("skeleton too large".into()))?;
        let atlas_len = c_int::try_from(atlas_data.len())
            .map_err(|_| SpineError::Invalid("atlas too large".into()))?;
        data.push(0);
        let mut error = [0 as c_char; 1024];
        let raw = {
            let _guard = LOAD_LOCK
                .lock()
                .map_err(|_| SpineError::Invalid("model loader lock is poisoned".into()))?;
            // Input buffers stay alive for the synchronous parser; it owns all
            // returned data. The bridge always NUL-terminates error messages.
            unsafe {
                ark_spine_load(
                    atlas_data.as_ptr().cast(),
                    atlas_len,
                    data.as_ptr(),
                    data_len,
                    json.into(),
                    error.as_mut_ptr(),
                    error.len() as c_int,
                )
            }
        };
        let raw = NonNull::new(raw).ok_or_else(|| {
            SpineError::Invalid(
                unsafe { CStr::from_ptr(error.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            )
        })?;
        let mut result = Self {
            raw,
            animations: Vec::new(),
            pages: Vec::new(),
            version,
        };
        // Indices come exclusively from counts supplied by this owned runtime.
        unsafe {
            for i in 0..ark_spine_animation_count(raw.as_ptr()) {
                result.animations.push(
                    CStr::from_ptr(ark_spine_animation_name(raw.as_ptr(), i))
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            for i in 0.. {
                let page = ark_spine_page(raw.as_ptr(), i);
                if page.is_null() {
                    break;
                }
                let name = CStr::from_ptr(page).to_string_lossy();
                let relative = Path::new(name.as_ref());
                if relative.is_absolute()
                    || relative.components().any(|c| {
                        !matches!(
                            c,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    })
                {
                    return Err(SpineError::Invalid(format!(
                        "invalid atlas page path: {name}"
                    )));
                }
                result
                    .pages
                    .push(atlas.parent().unwrap_or(Path::new(".")).join(relative));
            }
        }
        if result.animations.is_empty() || result.pages.is_empty() {
            return Err(SpineError::Invalid(
                "skeleton has no animations or atlas pages".into(),
            ));
        }
        Ok(result)
    }

    pub fn play(&mut self, name: &str, looping: bool) -> Result<(), SpineError> {
        let c_name =
            CString::new(name).map_err(|_| SpineError::Invalid("animation contains NUL".into()))?;
        if unsafe { ark_spine_play(self.raw.as_ptr(), c_name.as_ptr(), looping.into()) } == 0 {
            return Err(SpineError::Invalid(format!("animation not found: {name}")));
        }
        Ok(())
    }

    /// Returns the animation's duration in seconds without changing its state.
    pub fn animation_duration(&self, name: &str) -> Result<f32, SpineError> {
        let c_name =
            CString::new(name).map_err(|_| SpineError::Invalid("animation contains NUL".into()))?;
        let mut duration = 0.0;
        if unsafe {
            ark_spine_animation_duration(self.raw.as_ptr(), c_name.as_ptr(), &mut duration)
        } == 0
        {
            return Err(SpineError::Invalid(format!("animation not found: {name}")));
        }
        if !duration.is_finite() || duration < 0.0 {
            return Err(SpineError::Invalid(format!(
                "invalid animation duration for {name}: {duration}"
            )));
        }
        Ok(duration)
    }

    /// Replaces the skeleton pose with an unblended sample for load-time analysis.
    ///
    /// Each sample starts from the setup pose. AnimationState clocks, tracks,
    /// crossfades, and completion events stay unchanged; this is not a playback
    /// seek. After analysis, sample the initial animation at zero to restore its
    /// starting pose before regular playback. Invalid requests preserve the pose.
    pub fn sample_animation(&mut self, name: &str, time: f32) -> Result<(), SpineError> {
        if !time.is_finite() || time < 0.0 {
            return Err(SpineError::Invalid(
                "animation sample time must be finite and non-negative".into(),
            ));
        }
        let c_name =
            CString::new(name).map_err(|_| SpineError::Invalid("animation contains NUL".into()))?;
        match unsafe { ark_spine_sample_animation(self.raw.as_ptr(), c_name.as_ptr(), time) } {
            1 => Ok(()),
            0 => Err(SpineError::Invalid(format!("animation not found: {name}"))),
            _ => Err(SpineError::Invalid(format!(
                "animation sample time {time} is outside the valid duration for {name}"
            ))),
        }
    }

    /// Starts an animation using this duration for its incoming crossfade.
    ///
    /// Spine captures the duration when creating the track entry, so changing
    /// the default during a later update cannot configure this transition.
    pub fn play_with_mix(&mut self, name: &str, looping: bool, mix: f32) -> Result<(), SpineError> {
        if !mix.is_finite() || !(0.0..=2.0).contains(&mix) {
            return Err(SpineError::Invalid(
                "animation mix must be finite and between 0 and 2 seconds".into(),
            ));
        }
        let c_name =
            CString::new(name).map_err(|_| SpineError::Invalid("animation contains NUL".into()))?;
        if unsafe {
            ark_spine_play_with_mix(self.raw.as_ptr(), c_name.as_ptr(), looping.into(), mix)
        } == 0
        {
            return Err(SpineError::Invalid(format!("animation not found: {name}")));
        }
        Ok(())
    }

    pub fn update(&mut self, delta: f32, mix: f32) -> bool {
        unsafe {
            ark_spine_update(
                self.raw.as_ptr(),
                delta.clamp(0.0, 0.1),
                mix.clamp(0.0, 2.0),
            ) != 0
        }
    }

    pub fn meshes(&mut self, output: &mut Vec<MeshPart>) {
        struct Output<'a> {
            parts: &'a mut Vec<MeshPart>,
            count: usize,
        }
        unsafe extern "C" fn receive(
            context: *mut c_void,
            page: c_int,
            blend: c_int,
            positions: *const f32,
            uvs: *const f32,
            count: c_int,
            indices: *const u16,
            index_count: c_int,
            r: f32,
            g: f32,
            b: f32,
            a: f32,
        ) {
            // Buffers are borrowed only for this callback. Copy into reusable
            // vectors so no C-owned pointer can escape to a later frame.
            let output = unsafe { &mut *context.cast::<Output<'_>>() };
            if output.count == output.parts.len() {
                output.parts.push(MeshPart::default());
            }
            let part = &mut output.parts[output.count];
            part.page = page as usize;
            part.blend = blend;
            part.color = [r, g, b, a];
            part.positions.clear();
            part.uvs.clear();
            part.indices.clear();
            part.positions.extend_from_slice(unsafe {
                std::slice::from_raw_parts(positions.cast::<[f32; 2]>(), count as usize)
            });
            part.uvs.extend_from_slice(unsafe {
                std::slice::from_raw_parts(uvs.cast::<[f32; 2]>(), count as usize)
            });
            part.indices.extend_from_slice(unsafe {
                std::slice::from_raw_parts(indices, index_count as usize)
            });
            output.count += 1;
        }
        let mut target = Output {
            parts: output,
            count: 0,
        };
        unsafe {
            ark_spine_meshes(
                self.raw.as_ptr(),
                (&mut target as *mut Output<'_>).cast(),
                receive,
            )
        };
        target.parts.truncate(target.count);
    }
}

impl Drop for Spine {
    fn drop(&mut self) {
        unsafe { ark_spine_free(self.raw.as_ptr()) }
    }
}

fn binary_string<'a>(input: &mut &'a [u8]) -> Result<&'a str, SpineError> {
    let fail = || SpineError::Invalid("truncated Spine binary header".into());
    let mut size = 0usize;
    for shift in (0..35).step_by(7) {
        let (&byte, rest) = input.split_first().ok_or_else(fail)?;
        *input = rest;
        size |= usize::from(byte & 127) << shift;
        if byte & 128 == 0 {
            let length = size.saturating_sub(1);
            let value = input.get(..length).ok_or_else(fail)?;
            *input = &input[length..];
            return std::str::from_utf8(value).map_err(|_| fail());
        }
    }
    Err(fail())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_model(id: &str) -> Spine {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/models")
            .join(id);
        Spine::load(
            &root.join(format!("build_char_{id}.skel")),
            &root.join(format!("build_char_{id}.atlas")),
        )
        .unwrap()
    }

    fn relaxed_model(id: &str, mix: f32) -> Spine {
        let mut spine = load_model(id);
        spine.play("Relax", true).unwrap();
        for _ in 0..6 {
            spine.update(0.1, mix);
        }
        spine
    }

    fn positions(spine: &mut Spine) -> Vec<[f32; 2]> {
        let mut meshes = Vec::new();
        spine.meshes(&mut meshes);
        meshes.into_iter().flat_map(|mesh| mesh.positions).collect()
    }

    fn other_animation(spine: &Spine) -> String {
        spine
            .animations
            .iter()
            .find(|name| name.as_str() == "Move")
            .or_else(|| {
                spine
                    .animations
                    .iter()
                    .find(|name| name.as_str() != "Relax")
            })
            .expect("real model must have an animation other than Relax")
            .clone()
    }

    #[test]
    fn real_animation_samples_are_repeatable_and_independent_of_previous_poses() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let mut spine = relaxed_model(id, 0.3);
            let before_duration = positions(&mut spine);
            let duration = spine.animation_duration("Relax").unwrap();
            assert!(duration.is_finite() && duration > 0.0, "{id}");
            assert_eq!(positions(&mut spine), before_duration);
            let time = duration * 0.25;
            spine.sample_animation("Relax", time).unwrap();
            let expected = positions(&mut spine);
            assert!(expected.len() > 20);
            for name in spine.animations.clone() {
                let other_duration = spine.animation_duration(&name).unwrap();
                spine.sample_animation(&name, other_duration).unwrap();
                spine.sample_animation("Relax", time).unwrap();
                assert_eq!(
                    positions(&mut spine),
                    expected,
                    "{id}: previous {name} sample changed the Relax pose"
                );
            }
            spine.sample_animation("Relax", 0.0).unwrap();
            assert_ne!(positions(&mut spine), expected, "{id}: sample did not move");
        }
    }

    #[test]
    fn invalid_animation_samples_preserve_real_model_pose() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let mut spine = relaxed_model(id, 0.3);
            let duration = spine.animation_duration("Relax").unwrap();
            let before = positions(&mut spine);
            for (name, time) in [
                ("Relax", f32::NAN),
                ("Relax", f32::INFINITY),
                ("Relax", f32::NEG_INFINITY),
                ("Relax", -0.01),
                ("Relax", duration + 0.01),
                ("missing-animation", 0.0),
                ("invalid\0animation", 0.0),
            ] {
                assert!(spine.sample_animation(name, time).is_err());
                assert_eq!(positions(&mut spine), before, "{id}: {name} at {time}");
            }
            for name in ["missing-animation", "invalid\0animation"] {
                assert!(spine.animation_duration(name).is_err());
                assert_eq!(positions(&mut spine), before);
            }
        }
    }

    #[test]
    fn sampling_preserves_real_model_playback_clock_and_completion() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let assert_pose_matches = |spine: &mut Spine, reference: &mut Spine| {
                let actual = positions(spine);
                let expected = positions(reference);
                assert_eq!(actual.len(), expected.len(), "{id}");
                // Playback's FIRST blend computes old + (target - old), unlike
                // a SETUP sample. Resetting the old pose changes f32 rounding.
                for (index, (a, b)) in actual
                    .iter()
                    .flatten()
                    .zip(expected.iter().flatten())
                    .enumerate()
                {
                    assert!((a - b).abs() < 0.001, "{id}: component {index}: {a} != {b}");
                }
            };
            let mut spine = load_model(id);
            let mut reference = load_model(id);
            spine.play("Relax", false).unwrap();
            reference.play("Relax", false).unwrap();
            for _ in 0..2 {
                spine.update(0.1, 0.3);
                reference.update(0.1, 0.3);
            }
            let next = other_animation(&spine);
            spine
                .sample_animation(&next, spine.animation_duration(&next).unwrap())
                .unwrap();
            spine.sample_animation("Relax", 0.0).unwrap();
            spine.update(0.0, 0.3);
            reference.update(0.0, 0.3);
            assert_pose_matches(&mut spine, &mut reference);
            let before = positions(&mut spine);
            let steps = (spine.animation_duration("Relax").unwrap() / 0.1).ceil() as usize;
            let mut completed = false;
            for _ in 0..=steps {
                let actual_completed = spine.update(0.1, 0.3);
                assert_eq!(actual_completed, reference.update(0.1, 0.3), "{id}");
                completed |= actual_completed;
                assert_pose_matches(&mut spine, &mut reference);
            }
            assert!(completed, "{id}: completion event was lost");
            assert_ne!(positions(&mut spine), before, "{id}: playback stopped");
        }
    }

    #[test]
    fn first_transition_uses_its_requested_mix_for_real_models() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let mut immediate_positions = None;
            for mix in [0.0, 0.6, 2.0] {
                let mut spine = relaxed_model(id, 0.3);
                // The legacy path has the requested default before switching;
                // play_with_mix must match it without a preceding update.
                let mut reference = relaxed_model(id, mix);
                let next = other_animation(&spine);
                assert_eq!(positions(&mut spine), positions(&mut reference));
                spine.play_with_mix(&next, true, mix).unwrap();
                reference.play(&next, true).unwrap();
                for delta in [0.0, 0.05] {
                    // A later default change must not alter an existing fade.
                    spine.update(delta, 0.3);
                    reference.update(delta, 0.3);
                    let actual = positions(&mut spine);
                    assert!(actual.len() > 20, "{id} produced no geometry");
                    assert_eq!(
                        actual,
                        positions(&mut reference),
                        "{id}: incorrect first transition at mix={mix}, delta={delta}"
                    );
                    if delta > 0.0 {
                        if mix == 0.0 {
                            immediate_positions = Some(actual);
                        } else {
                            assert_ne!(
                                Some(actual),
                                immediate_positions,
                                "{id}: crossfade must differ from an immediate change"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejected_transition_preserves_real_model_playback_and_default_mix() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            for (name, mix) in [
                (None, f32::NAN),
                (None, f32::INFINITY),
                (None, f32::NEG_INFINITY),
                (None, -0.01),
                (None, 2.01),
                (Some("missing-animation"), 1.5),
                (Some("invalid\0animation"), 1.5),
            ] {
                let mut spine = relaxed_model(id, 0.3);
                let mut reference = relaxed_model(id, 0.3);
                let next = other_animation(&spine);
                assert!(spine
                    .play_with_mix(name.unwrap_or(&next), true, mix)
                    .is_err());
                assert_eq!(positions(&mut spine), positions(&mut reference));
                // No update between rejection and play: this also detects an
                // invalid request that changed only the default mix duration.
                spine.play(&next, true).unwrap();
                reference.play(&next, true).unwrap();
                for delta in [0.0, 0.05] {
                    spine.update(delta, 0.3);
                    reference.update(delta, 0.3);
                    assert_eq!(
                        positions(&mut spine),
                        positions(&mut reference),
                        "{id}: rejected transition changed playback for {name:?}, mix={mix}"
                    );
                }

                let mut spine = relaxed_model(id, 0.3);
                let mut reference = relaxed_model(id, 0.3);
                assert!(spine
                    .play_with_mix(name.unwrap_or(&next), true, mix)
                    .is_err());
                spine.update(0.05, 0.3);
                reference.update(0.05, 0.3);
                assert_eq!(
                    positions(&mut spine),
                    positions(&mut reference),
                    "{id}: rejected transition must keep playing Relax"
                );
            }
        }
    }

    #[test]
    fn bundled_catalog_plays_raw_clips_without_name_classification() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = crate::ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        for model in &catalog.models {
            let paths = model.resource_paths(&root);
            let mut spine = Spine::load(&paths[0], &paths[1]).unwrap();
            let mut animated = false;
            for name in spine.animations.clone() {
                let duration = spine.animation_duration(&name).unwrap();
                assert!(
                    duration.is_finite() && duration >= 0.0,
                    "{}: {name}",
                    model.id
                );
                if duration == 0.0 {
                    spine.sample_animation(&name, 0.0).unwrap();
                    continue;
                }
                spine.play_with_mix(&name, true, 0.0).unwrap();
                spine.update(0.0, 0.0);
                let before = positions(&mut spine);
                let mut changed = false;
                let mut completed = 0;
                for _ in 0..(duration * 60.0 * 2.5).ceil() as usize {
                    completed += usize::from(spine.update(1.0 / 60.0, 0.0));
                    let after = positions(&mut spine);
                    assert!(after.iter().flatten().all(|value| value.is_finite()));
                    changed |= before != after;
                }
                assert!(
                    completed >= 2,
                    "{}: {name} did not animate through multiple loops",
                    model.id
                );
                animated |= changed;
            }
            assert!(animated, "{}: no animated geometry", model.id);
            if spine.animations.iter().any(|name| name == "Start") {
                let duration = spine.animation_duration("Start").unwrap();
                spine.play_with_mix("Start", false, 0.0).unwrap();
                let mut completed = 0;
                for _ in 0..(duration * 60.0 * 2.5 + 2.0).ceil() as usize {
                    completed += usize::from(spine.update(1.0 / 60.0, 0.0));
                }
                assert_eq!(completed, 1, "{}: entrance must play once", model.id);
            }
        }
    }

    #[test]
    fn real_models_load_and_produce_animated_meshes() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let mut spine = load_model(id);
            assert!(spine.pages.iter().all(|path| path.is_file()));
            assert!(spine.animations.iter().any(|name| name == "Relax"));
            spine.play("Relax", true).unwrap();
            spine.update(0.0, 0.3);
            let mut meshes = Vec::new();
            spine.meshes(&mut meshes);
            let before: Vec<_> = meshes
                .iter()
                .flat_map(|mesh| mesh.positions.iter().copied())
                .collect();
            assert!(before.len() > 20, "{id} produced no geometry");
            for _ in 0..30 {
                spine.update(1.0 / 60.0, 0.3);
            }
            spine.meshes(&mut meshes);
            let after: Vec<_> = meshes
                .iter()
                .flat_map(|mesh| mesh.positions.iter().copied())
                .collect();
            assert_ne!(before, after, "{id} animation did not advance");
            assert!(meshes.iter().all(|mesh| mesh
                .indices
                .iter()
                .all(|i| usize::from(*i) < mesh.positions.len())));
        }
    }

    #[test]
    fn lappland_battle_attacks_use_real_animated_geometry() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = crate::ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        let model = catalog
            .models
            .iter()
            .find(|model| model.id == "whitw2-battle")
            .unwrap();
        model.validate(Some(&root)).unwrap();
        let paths = model.resource_paths(&root);
        let mut spine = Spine::load(&paths[0], &paths[1]).unwrap();
        assert!(spine.pages.iter().all(|path| path.is_file()));
        assert!(spine.animations.iter().any(|name| name == "Idle"));
        let duration = spine.animation_duration("Attack").unwrap();
        assert!(duration > 0.0);
        spine.play_with_mix("Attack", true, 0.0).unwrap();
        spine.update(0.0, 0.0);
        let before = positions(&mut spine);
        assert!(before.len() > 20);
        let mut completed = 0;
        let mut changed = false;
        for _ in 0..(duration * 60.0 * 2.5).ceil() as usize {
            completed += usize::from(spine.update(1.0 / 60.0, 0.0));
            let after = positions(&mut spine);
            assert!(after.iter().flatten().all(|value| value.is_finite()));
            changed |= before != after;
        }
        assert!(changed, "Attack must advance the real skeleton");
        assert!(
            completed >= 2,
            "manual Attack must support repeated playback"
        );
    }

    #[test]
    fn real_model_moves_between_threads_and_keeps_animating() {
        let mut spine = load_model("4058_pepe");
        spine.play("Relax", true).unwrap();
        let mut meshes = Vec::new();
        spine.meshes(&mut meshes);
        let before: Vec<_> = meshes
            .iter()
            .flat_map(|mesh| mesh.positions.iter().copied())
            .collect();
        let spine = std::thread::spawn(move || {
            for _ in 0..30 {
                spine.update(1.0 / 60.0, 0.3);
            }
            spine.meshes(&mut meshes);
            let after: Vec<_> = meshes
                .iter()
                .flat_map(|mesh| mesh.positions.iter().copied())
                .collect();
            assert_ne!(before, after);
            spine
        })
        .join()
        .unwrap();
        std::thread::spawn(move || drop(spine)).join().unwrap();
    }

    #[test]
    fn real_models_load_and_animate_on_concurrent_threads() {
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    for _ in 0..4 {
                        let mut spine = load_model(id);
                        spine.play("Relax", true).unwrap();
                        let mut meshes = Vec::new();
                        for _ in 0..30 {
                            spine.update(1.0 / 60.0, 0.3);
                            spine.meshes(&mut meshes);
                            assert!(!meshes.is_empty());
                            assert!(meshes.iter().all(|mesh| mesh.positions.iter().all(
                                |position| position.iter().all(|component| component.is_finite())
                            )));
                        }
                    }
                });
            }
        });
    }

    #[test]
    fn truncated_header_is_rejected_before_ffi() {
        assert!(binary_string(&mut &[255u8][..]).is_err());
    }
}

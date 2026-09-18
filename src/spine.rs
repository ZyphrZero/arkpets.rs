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
    fn ark_spine_page(pet: *mut c_void, index: c_int) -> *const c_char;
    fn ark_spine_play(pet: *mut c_void, name: *const c_char, looping: c_int) -> c_int;
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
// keeps Spine !Sync; every runtime operation requires &mut self or ownership.
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

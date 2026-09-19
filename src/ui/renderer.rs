use arkpets::{
    spine::{MeshPart, Spine},
    CharacterModel,
};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Vec2};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

mod bounds;
#[cfg(windows)]
mod glow;
mod wgpu;

#[derive(Debug, Clone, Copy)]
pub struct Effects {
    pub opacity: f32,
    /// Straight RGBA, with the outline condition's fade applied to alpha.
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub shadow_opacity: f32,
    pub high_quality: bool,
    pub mipmaps: bool,
}

impl Effects {
    #[cfg(test)]
    pub fn plain(opacity: f32) -> Self {
        Self {
            opacity,
            outline_color: [0.0; 4],
            outline_width: 0.0,
            shadow_opacity: 0.0,
            high_quality: true,
            mipmaps: false,
        }
    }
}

fn mipmaps(image: &ColorImage) -> Vec<ColorImage> {
    let bytes: Vec<_> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    let mut source = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
        .expect("decoded atlas has valid dimensions");
    let mut levels = vec![image.clone()];
    while source.width() > 1 || source.height() > 1 {
        source = image::imageops::resize(
            &source,
            (source.width() / 2).max(1),
            (source.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
        // Filter premultiplied colors so transparent pixels cannot bleed their RGB.
        levels.push(ColorImage::from_rgba_premultiplied(
            [source.width() as usize, source.height() as usize],
            source.as_raw(),
        ));
    }
    levels
}

/// Backend resources outlive model selection and are recreated with the GPU context.
pub enum Compositor {
    Wgpu(wgpu::Compositor),
    #[cfg(windows)]
    Glow(glow::Compositor),
    #[cfg(test)]
    Recording,
}

impl Compositor {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        if let Some(state) = &cc.wgpu_render_state {
            return Self::Wgpu(wgpu::Compositor::new(state));
        }
        #[cfg(windows)]
        if let Some(gl) = &cc.gl {
            return Self::Glow(glow::Compositor::new(gl.clone()));
        }
        #[cfg(test)]
        return Self::Recording;
        #[cfg(not(test))]
        panic!("A GPU context is required to composite the pet");
    }

    fn paint(
        &self,
        painter: &egui::Painter,
        meshes: Vec<egui::Mesh>,
        images: Arc<Vec<ColorImage>>,
        effects: Effects,
    ) {
        match self {
            Self::Wgpu(compositor) => compositor.paint(painter, meshes, images, effects),
            #[cfg(windows)]
            Self::Glow(compositor) => compositor.paint(painter, meshes, images, effects),
            #[cfg(test)]
            Self::Recording => {
                // Headless tests inspect the submitted group, not fabricated GPU pixels.
                if let Some(rect) = group_rect(painter, &meshes, &effects) {
                    painter.add(egui::PaintCallback {
                        rect,
                        callback: Arc::new(RecordedGroup {
                            meshes,
                            opacity: effects.opacity,
                        }),
                    });
                }
            }
        }
    }

    #[cfg(windows)]
    pub fn destroy_gl(&self) {
        if let Self::Glow(compositor) = self {
            compositor.destroy();
        }
    }
}

#[cfg(test)]
pub struct RecordedGroup {
    pub meshes: Vec<egui::Mesh>,
    pub opacity: f32,
}

fn group_rect(painter: &egui::Painter, meshes: &[egui::Mesh], effects: &Effects) -> Option<Rect> {
    let mut rect = Rect::NOTHING;
    for vertex in meshes.iter().flat_map(|mesh| &mesh.vertices) {
        rect.extend_with(vertex.pos);
    }
    let scale = painter.ctx().pixels_per_point();
    let outline_radius = if effects.outline_color[3] > 0.0 {
        effects.outline_width * if effects.high_quality { 2.0 } else { 1.0 }
    } else {
        0.0
    };
    let shadow_radius = if effects.shadow_opacity > 0.0 {
        scale * if effects.high_quality { 4.0 } else { 2.0 }
    } else {
        0.0
    };
    rect = rect
        .expand((outline_radius.max(shadow_radius) + 1.0) / scale)
        .intersect(painter.clip_rect())
        .intersect(painter.ctx().input(|input| input.viewport_rect()));
    if !rect.is_finite() || !rect.is_positive() {
        return None;
    }
    Some(Rect::from_min_max(
        (rect.min * scale).floor() / scale,
        (rect.max * scale).ceil() / scale,
    ))
}

pub struct Renderer {
    pub spine: Spine,
    images: Arc<Vec<ColorImage>>,
    meshes: Vec<MeshPart>,
    pub bounds: Rect,
    reference_height: f32,
    pub vertices: usize,
    pub triangles: usize,
    skeleton_path: PathBuf,
    atlas_path: PathBuf,
    resource_versions: Vec<ResourceVersion>,
    pub canvas_settings: arkpets::config::CanvasSettings,
    pub effective_canvas_coverage: f32,
    pub canvas_samples: usize,
}

impl Renderer {
    pub fn load(
        model: &CharacterModel,
        root: &Path,
        canvas: arkpets::config::CanvasSettings,
    ) -> Result<Self, String> {
        if model
            .resource_path
            .as_deref()
            .is_some_and(|path| path.contains("://"))
        {
            return Err("当前仅支持本地模型目录，请先下载资源".into());
        }
        let root = root.join(model.resource_path.as_deref().unwrap_or(""));
        let skeleton_path = root.join(&model.skeleton);
        let atlas_path = root.join(&model.atlas);
        let mut spine = Spine::load(&skeleton_path, &atlas_path).map_err(|e| e.to_string())?;
        let initial = spine
            .animations
            .iter()
            .find(|name| matches!(name.as_str(), "Relax" | "Idle" | "Stand"))
            .unwrap_or(&spine.animations[0])
            .clone();
        spine.play(&initial, true).map_err(|e| e.to_string())?;
        spine.update(0.0, 0.3);
        let mut meshes = Vec::new();
        spine.meshes(&mut meshes);
        let mut bounds = Rect::NOTHING;
        for part in &meshes {
            for &[x, y] in &part.positions {
                bounds.extend_with(Pos2::new(x, y));
            }
        }
        if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return Err("模型没有可渲染的几何数据".into());
        }
        let images = spine
            .pages
            .iter()
            .map(|path| {
                let image = image::open(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .into_rgba8();
                Ok(ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let calculation =
            calculate_bounds(&skeleton_path, &atlas_path, &spine.pages, &images, canvas)?;
        let resource_versions = resource_versions(&skeleton_path, &atlas_path, &spine.pages)?;
        let mut result = Self {
            spine,
            images: Arc::new(images),
            meshes,
            bounds: calculation.rect,
            reference_height: bounds.height(),
            vertices: 0,
            triangles: 0,
            skeleton_path,
            atlas_path,
            resource_versions,
            canvas_settings: canvas,
            effective_canvas_coverage: calculation.effective_coverage,
            canvas_samples: calculation.samples,
        };
        result.update(0.0, 0.3)?;
        Ok(result)
    }

    pub fn recalculate_bounds(
        &mut self,
        settings: arkpets::config::CanvasSettings,
    ) -> Result<(), String> {
        if self.resource_versions
            != resource_versions(&self.skeleton_path, &self.atlas_path, &self.spine.pages)?
        {
            return Err("模型文件已在磁盘更改，请重新载入模型后计算边界".into());
        }
        let calculation = calculate_bounds(
            &self.skeleton_path,
            &self.atlas_path,
            &self.spine.pages,
            &self.images,
            settings,
        )?;
        self.bounds = calculation.rect;
        self.canvas_settings = settings;
        self.effective_canvas_coverage = calculation.effective_coverage;
        self.canvas_samples = calculation.samples;
        Ok(())
    }

    pub fn update(&mut self, dt: f32, mix: f32) -> Result<bool, String> {
        let completed = self.spine.update(dt, mix);
        self.spine.meshes(&mut self.meshes);
        if self.meshes.iter().any(|mesh| mesh.blend > 1) {
            return Err("该模型使用尚未支持的 Multiply / Screen 混合模式".into());
        }
        self.vertices = self.meshes.iter().map(|mesh| mesh.positions.len()).sum();
        self.triangles = self.meshes.iter().map(|mesh| mesh.indices.len() / 3).sum();
        Ok(completed)
    }

    pub fn size(&self, height: f32) -> Vec2 {
        // Cropping changes the canvas, not the skeleton's world-to-screen scale.
        self.bounds.size() * (height / self.reference_height)
    }

    #[cfg(test)]
    pub fn paint(
        &self,
        painter: &egui::Painter,
        target: Rect,
        direction: arkpets::Direction,
        opacity: f32,
        compositor: &Compositor,
    ) {
        self.paint_with_facing(
            painter,
            target,
            if direction == arkpets::Direction::Left {
                -1.0
            } else {
                1.0
            },
            Effects::plain(opacity),
            compositor,
        );
    }

    pub fn paint_with_facing(
        &self,
        painter: &egui::Painter,
        target: Rect,
        facing: f32,
        effects: Effects,
        compositor: &Compositor,
    ) {
        compositor.paint(
            painter,
            mesh_batches(&self.meshes, self.bounds, target, facing).collect(),
            self.images.clone(),
            effects,
        );
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BoundsKey {
    resources: Vec<ResourceVersion>,
    coverage: u32,
    interval: u32,
}

type CachedBounds = Arc<OnceLock<Result<bounds::Calculation, String>>>;
type ResourceVersion = (PathBuf, u64, std::time::SystemTime);

fn resource_versions(
    skeleton: &Path,
    atlas: &Path,
    pages: &[PathBuf],
) -> Result<Vec<ResourceVersion>, String> {
    [skeleton, atlas]
        .into_iter()
        .chain(pages.iter().map(PathBuf::as_path))
        .map(|path| {
            let metadata =
                std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
            Ok((
                path.to_owned(),
                metadata.len(),
                metadata.modified().map_err(|error| error.to_string())?,
            ))
        })
        .collect()
}

fn calculate_bounds(
    skeleton: &Path,
    atlas: &Path,
    pages: &[PathBuf],
    images: &[ColorImage],
    settings: arkpets::config::CanvasSettings,
) -> Result<bounds::Calculation, String> {
    static CACHE: OnceLock<Mutex<HashMap<BoundsKey, CachedBounds>>> = OnceLock::new();
    let resources = resource_versions(skeleton, atlas, pages)?;
    let key = BoundsKey {
        resources,
        coverage: settings.coverage.to_bits(),
        interval: settings.sampling_interval,
    };
    let entry = {
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "边界缓存锁已失效".to_owned())?;
        if cache.len() >= 32 && !cache.contains_key(&key) {
            if let Some(expired) = cache.keys().next().cloned() {
                cache.remove(&expired);
            }
        }
        cache.entry(key).or_default().clone()
    };
    entry
        .get_or_init(|| {
            // Sampling never mutates the visible animation or its paused state.
            let mut sampler = Spine::load(skeleton, atlas).map_err(|error| error.to_string())?;
            bounds::calculate(&mut sampler, images, &settings)
        })
        .clone()
}

fn mesh_batches<'a>(
    parts: &'a [MeshPart],
    bounds: Rect,
    target: Rect,
    facing: f32,
) -> impl Iterator<Item = egui::Mesh> + 'a {
    let scale = target.height() / bounds.height();
    // Only adjacent slots may be merged: sorting by texture changes occlusion.
    // Additive blending is encoded in each vertex's premultiplied color.
    parts.chunk_by(|a, b| a.page == b.page).map(move |batch| {
        let mut mesh = egui::Mesh::with_texture(egui::TextureId::Managed(batch[0].page as u64 + 1));
        mesh.vertices
            .reserve_exact(batch.iter().map(|part| part.positions.len()).sum());
        mesh.indices
            .reserve_exact(batch.iter().map(|part| part.indices.len()).sum());
        for part in batch {
            let index_offset = mesh.vertices.len() as u32;
            let [r, g, b, a] = part.color;
            // Global opacity belongs to the composed image, never to individual slots.
            let alpha = (a * 255.0).clamp(0.0, 255.0) as u8;
            let mut color = Color32::from_rgba_unmultiplied(
                (r * 255.0) as u8,
                (g * 255.0) as u8,
                (b * 255.0) as u8,
                alpha,
            );
            if part.blend == 1 {
                color = Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 0);
            }
            mesh.vertices
                .extend(part.positions.iter().zip(&part.uvs).map(|(pos, uv)| {
                    let x = (pos[0] - bounds.min.x) * scale;
                    egui::epaint::Vertex {
                        pos: Pos2::new(
                            target.center().x + (x - target.width() * 0.5) * facing,
                            target.top() + (bounds.max.y - pos[1]) * scale,
                        ),
                        uv: Pos2::new(uv[0], uv[1]),
                        color,
                    }
                }));
            mesh.indices
                .extend(part.indices.iter().map(|i| index_offset + u32::from(*i)));
        }
        mesh
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkpets::Direction;

    #[test]
    fn mipmaps_keep_the_base_and_filter_premultiplied_color_without_dark_edges() {
        let image = ColorImage::from_rgba_unmultiplied(
            [2, 2],
            &[255, 255, 255, 255, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0],
        );
        let levels = mipmaps(&image);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0], image);
        assert_eq!(levels[1].size, [1, 1]);
        let color = levels[1].pixels[0];
        assert!((63..=65).contains(&color.a()));
        assert_eq!(color.r(), color.a());
        assert_eq!(color.g(), color.a());
        assert_eq!(color.b(), color.a());
        let odd = ColorImage::filled([3, 5], Color32::WHITE);
        let levels = mipmaps(&odd);
        assert_eq!(
            levels.iter().map(|image| image.size).collect::<Vec<_>>(),
            [[3, 5], [1, 2], [1, 1]]
        );
        assert!(levels
            .iter()
            .all(|image| image.pixels.iter().all(|pixel| *pixel == Color32::WHITE)));
    }

    #[test]
    fn recalculating_canvas_keeps_the_live_animation_and_atlas() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = arkpets::ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        let mut renderer = Renderer::load(&catalog.models[0], &root, Default::default()).unwrap();
        renderer.update(0.1, 0.3).unwrap();
        let before: Vec<_> = renderer
            .meshes
            .iter()
            .flat_map(|mesh| mesh.positions.iter().copied())
            .collect();
        let atlas = renderer.images.clone();
        let old_bounds = renderer.bounds;
        let old_scale = renderer.size(180.0).y / renderer.bounds.height();
        renderer
            .recalculate_bounds(arkpets::config::CanvasSettings {
                coverage: 0.45,
                sampling_interval: 4,
            })
            .unwrap();
        assert_ne!(renderer.bounds, old_bounds);
        assert!((renderer.size(180.0).y / renderer.bounds.height() - old_scale).abs() < 0.000001);
        assert!(Arc::ptr_eq(&atlas, &renderer.images));
        let mut pose = Vec::new();
        renderer.spine.meshes(&mut pose);
        assert_eq!(
            pose.into_iter()
                .flat_map(|mesh| mesh.positions)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn real_models_submit_global_opacity_separately_from_slot_colors() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = arkpets::ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        for model in &catalog.models {
            let ctx = egui::Context::default();
            let mut renderer = Renderer::load(model, &root, Default::default()).unwrap();
            renderer.update(0.1, 0.3).unwrap();
            let mut original = None;
            for opacity in [1.0, 0.25, 0.0] {
                let output = ctx.run_ui(Default::default(), |ui| {
                    renderer.paint(
                        ui.painter(),
                        Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)),
                        Direction::Right,
                        opacity,
                        &Compositor::Recording,
                    );
                });
                let groups: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Callback(callback) => {
                            callback.callback.downcast_ref::<RecordedGroup>()
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(groups.len(), 1);
                assert_eq!(groups[0].opacity, opacity);
                let colors: Vec<_> = groups[0]
                    .meshes
                    .iter()
                    .flat_map(|mesh| mesh.vertices.iter().map(|v| v.color))
                    .collect();
                assert!(!colors.is_empty());
                if let Some(expected) = &original {
                    assert_eq!(&colors, expected);
                } else {
                    original = Some(colors);
                }
                output.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn batches_preserve_slot_order_indices_and_additive_colors() {
        let part = |page, blend, x| MeshPart {
            page,
            blend,
            positions: vec![[x, 0.0], [x + 1.0, 0.0], [x, 1.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            color: [1.0, 0.5, 0.25, 0.5],
        };
        let parts = [
            part(0, 0, 0.0),
            part(0, 1, 2.0),
            part(1, 0, 4.0),
            part(0, 0, 6.0),
        ];
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
        let target = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::splat(100.0));
        let meshes: Vec<_> = mesh_batches(&parts, bounds, target, 1.0).collect();
        assert_eq!(meshes.len(), 3);
        assert_eq!(
            meshes
                .iter()
                .map(|mesh| mesh.texture_id)
                .collect::<Vec<_>>(),
            [
                egui::TextureId::Managed(1),
                egui::TextureId::Managed(2),
                egui::TextureId::Managed(1)
            ]
        );
        assert!(meshes.iter().all(egui::Mesh::is_valid));
        assert_eq!(meshes[0].indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(meshes[1].indices, [0, 1, 2]);
        let color = Color32::from_rgba_unmultiplied(255, 127, 63, 127);
        assert_eq!(meshes[0].vertices[0].color, color);
        assert_eq!(
            meshes[0].vertices[3].color,
            Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 0)
        );
        assert_eq!(meshes[0].vertices[3].pos, Pos2::new(120.0, 200.0));
        assert_eq!(meshes[0].vertices[4].uv, Pos2::new(1.0, 0.0));
        assert_eq!(meshes[1].vertices[0].pos, Pos2::new(140.0, 200.0));
        assert_eq!(meshes[2].vertices[0].pos, Pos2::new(160.0, 200.0));

        let mirrored: Vec<_> = mesh_batches(&parts, bounds, target, -1.0).collect();
        assert_eq!(mirrored[0].vertices[3].pos, Pos2::new(180.0, 200.0));
        assert_eq!(mirrored[0].vertices[0].color.a(), 127);
        assert_eq!(mirrored[0].vertices[3].color.a(), 0);
    }
}

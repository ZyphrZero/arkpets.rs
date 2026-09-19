//! One-time CPU approximation of Java's accumulated-animation Alpha crop.
//!
//! Only the analysis mask is limited to 256 pixels on its longest side. Atlas
//! images stay at their original resolution and are sampled bilinearly. Unlike
//! Java's RGBA8 framebuffer and two-pixel edge scan, accumulation uses f32 and
//! examines every analysis pixel; this is not a pixel-identical GPU readback.

use arkpets::{
    config::CanvasSettings,
    spine::{MeshPart, Spine},
};
use eframe::egui::{ColorImage, Pos2, Rect};

const ANALYSIS_SIZE: usize = 256;
const MAX_SAMPLES: usize = 4096;
const MIN_ALPHA: f32 = 1.0 / 255.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct Calculation {
    pub rect: Rect,
    /// Coverage after fallback, before the usual 8-bit threshold rounding.
    pub effective_coverage: f32,
    pub samples: usize,
}

struct AnimationSamples {
    name: String,
    duration: f32,
    count: usize,
    step: f64,
}

impl AnimationSamples {
    fn time(&self, index: usize) -> f32 {
        if self.count == 1 {
            self.duration / 2.0
        } else {
            (index as f64 * self.step) as f32
        }
    }
}

fn sampling_plan(
    spine: &Spine,
    settings: &CanvasSettings,
) -> Result<Vec<AnimationSamples>, String> {
    if !settings.coverage.is_finite() || !(0.0..=1.0).contains(&settings.coverage) {
        return Err("窗口边界覆盖阈值必须是 0–1 之间的有限数值".into());
    }
    if settings.sampling_interval == 0 {
        return Err("窗口边界抽帧间隔必须大于 0".into());
    }
    let step = f64::from(settings.sampling_interval) / 30.0;
    let mut total = 0;
    let mut plan = Vec::with_capacity(spine.animations.len());
    for name in &spine.animations {
        let duration = spine.animation_duration(name).map_err(|e| e.to_string())?;
        if !duration.is_finite() || duration < 0.0 {
            return Err(format!("动画 {name} 的时长无效，无法计算窗口边界"));
        }
        let count = if f64::from(duration) <= 2.0 * step {
            1
        } else {
            (f64::from(duration) / step).ceil() as usize
        };
        if count > MAX_SAMPLES - total {
            return Err(format!(
                "窗口边界预计算超过 {MAX_SAMPLES} 个姿态，请选择更粗略的边界计算方式"
            ));
        }
        total += count;
        plan.push(AnimationSamples {
            name: name.clone(),
            duration,
            count,
            step,
        });
    }
    if total == 0 {
        return Err("模型没有可用于计算窗口边界的动画".into());
    }
    Ok(plan)
}

/// Samples all animations and returns the unpadded crop in skeleton coordinates.
///
/// Sampling changes the skeleton pose. Call this on a newly loaded analysis
/// instance, or explicitly restore the desired pose before displaying it. The
/// animation track's clock is never advanced by the sampling API.
pub(super) fn calculate(
    spine: &mut Spine,
    images: &[ColorImage],
    settings: &CanvasSettings,
) -> Result<Calculation, String> {
    let plan = sampling_plan(spine, settings)?;
    for image in images {
        if image.size.contains(&0)
            || image.size[0].checked_mul(image.size[1]) != Some(image.pixels.len())
        {
            return Err("图集尺寸或像素数据无效，无法计算窗口边界".into());
        }
    }
    let mut meshes = Vec::new();
    let mut geometry = Rect::NOTHING;
    for animation in &plan {
        for index in 0..animation.count {
            sample(spine, animation, index, &mut meshes)?;
            for mesh in &meshes {
                validate_mesh(mesh, images)?;
                for &[x, y] in &mesh.positions {
                    geometry.extend_with(Pos2::new(x, y));
                }
            }
        }
    }
    if !geometry.is_finite()
        || !geometry.is_positive()
        || !geometry.width().is_finite()
        || !geometry.height().is_finite()
    {
        return Err("模型采样没有有效的二维几何范围，无法计算窗口边界".into());
    }
    let mut mask = AlphaMask::new(geometry, settings.coverage);
    let total: usize = plan.iter().map(|animation| animation.count).sum();
    let sample_alpha = (1.0 - 10.0_f32.powf(-4.0 / total as f32)
        + 10.0_f32.powf(1.0 / total as f32 - 2.0))
    .clamp(MIN_ALPHA, 1.0);
    for animation in &plan {
        for index in 0..animation.count {
            sample(spine, animation, index, &mut meshes)?;
            for mesh in &meshes {
                // The compositor encodes additive slots with zero output Alpha.
                // Their RGB light cannot enlarge the visible coverage mask.
                if mesh.blend == 1 || mesh.color[3] <= 0.0 {
                    continue;
                }
                mask.draw(mesh, &images[mesh.page], sample_alpha);
            }
        }
    }
    let (rect, effective_coverage) = mask.crop(settings.coverage)?;
    Ok(Calculation {
        rect,
        effective_coverage,
        samples: total,
    })
}

fn sample(
    spine: &mut Spine,
    animation: &AnimationSamples,
    index: usize,
    meshes: &mut Vec<MeshPart>,
) -> Result<(), String> {
    spine
        .sample_animation(&animation.name, animation.time(index))
        .map_err(|e| e.to_string())?;
    spine.meshes(meshes);
    Ok(())
}

fn validate_mesh(mesh: &MeshPart, images: &[ColorImage]) -> Result<(), String> {
    if images.get(mesh.page).is_none()
        || mesh.positions.len() != mesh.uvs.len()
        || !mesh.indices.len().is_multiple_of(3)
        || mesh
            .indices
            .iter()
            .any(|&index| usize::from(index) >= mesh.positions.len())
        || mesh
            .positions
            .iter()
            .chain(&mesh.uvs)
            .flatten()
            .chain(&mesh.color)
            .any(|component| !component.is_finite())
    {
        return Err("模型采样包含无效的网格、图集索引或非有限坐标".into());
    }
    if !(0..=1).contains(&mesh.blend) {
        return Err("该模型使用尚未支持的 Multiply / Screen 混合模式".into());
    }
    Ok(())
}

struct AlphaMask {
    origin: Pos2,
    pixel_size: f64,
    width: usize,
    height: usize,
    cutoff: f32,
    pixels: Vec<f32>,
}

impl AlphaMask {
    fn new(geometry: Rect, coverage: f32) -> Self {
        let pixel_size = f64::from(geometry.width().max(geometry.height())) / ANALYSIS_SIZE as f64;
        let width = (f64::from(geometry.width()) / pixel_size).ceil() as usize;
        let height = (f64::from(geometry.height()) / pixel_size).ceil() as usize;
        Self {
            origin: geometry.min,
            pixel_size,
            width,
            height,
            cutoff: (coverage * 255.0).floor().max(1.0) / 255.0,
            pixels: vec![0.0; width * height],
        }
    }

    fn draw(&mut self, mesh: &MeshPart, image: &ColorImage, sample_alpha: f32) {
        let alpha = mesh.color[3].clamp(0.0, 1.0) * sample_alpha;
        for indices in mesh.indices.as_chunks::<3>().0 {
            let mut vertices = [[0.0; 2]; 3];
            let mut uvs = [[0.0; 2]; 3];
            for (i, &index) in indices.iter().enumerate() {
                let index = usize::from(index);
                vertices[i] = [
                    (f64::from(mesh.positions[index][0]) - f64::from(self.origin.x))
                        / self.pixel_size,
                    (f64::from(mesh.positions[index][1]) - f64::from(self.origin.y))
                        / self.pixel_size,
                ];
                uvs[i] = [f64::from(mesh.uvs[index][0]), f64::from(mesh.uvs[index][1])];
            }
            let area = Edge::new(vertices[0], vertices[1]).at(vertices[2]);
            if area == 0.0 {
                continue;
            }
            if area < 0.0 {
                vertices.swap(1, 2);
                uvs.swap(1, 2);
            }
            let edges = [
                Edge::new(vertices[1], vertices[2]),
                Edge::new(vertices[2], vertices[0]),
                Edge::new(vertices[0], vertices[1]),
            ];
            let area = area.abs();
            let min_x = vertices.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min);
            let max_x = vertices
                .iter()
                .map(|v| v[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = vertices.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min);
            let max_y = vertices
                .iter()
                .map(|v| v[1])
                .fold(f64::NEG_INFINITY, f64::max);
            for y in min_y.floor().max(0.0) as usize..(max_y.ceil() as usize).min(self.height) {
                for x in min_x.floor().max(0.0) as usize..(max_x.ceil() as usize).min(self.width) {
                    let pixel = y * self.width + x;
                    // Source-over Alpha only grows. Once a pixel meets the
                    // requested crop threshold, further samples cannot change
                    // its inclusion at that threshold or any fallback below it.
                    if self.pixels[pixel] >= self.cutoff {
                        continue;
                    }
                    let point = [x as f64 + 0.5, y as f64 + 0.5];
                    let weights = edges.map(|edge| edge.at(point));
                    if edges
                        .iter()
                        .zip(weights)
                        .any(|(edge, weight)| weight < 0.0 || (weight == 0.0 && !edge.inclusive))
                    {
                        continue;
                    }
                    let uv = [0, 1].map(|axis| {
                        (uvs[0][axis] * weights[0]
                            + uvs[1][axis] * weights[1]
                            + uvs[2][axis] * weights[2])
                            / area
                    });
                    let source_alpha = bilinear_alpha(image, uv) * alpha;
                    let target = &mut self.pixels[pixel];
                    *target += source_alpha * (1.0 - *target);
                }
            }
        }
    }

    fn crop(&self, coverage: f32) -> Result<(Rect, f32), String> {
        let mut threshold = coverage.max(MIN_ALPHA);
        loop {
            // Match the original threshold's 8-bit floor, without quantizing
            // every intermediate blend into a framebuffer byte.
            let threshold_byte = (threshold * 255.0).floor().max(1.0);
            let threshold_alpha = threshold_byte / 255.0;
            let mut min = [self.width, self.height];
            let mut max = [0, 0];
            for (index, &alpha) in self.pixels.iter().enumerate() {
                if alpha >= threshold_alpha {
                    let x = index % self.width;
                    let y = index / self.width;
                    min[0] = min[0].min(x);
                    min[1] = min[1].min(y);
                    max[0] = max[0].max(x + 1);
                    max[1] = max[1].max(y + 1);
                }
            }
            if min[0] < max[0] && min[1] < max[1] {
                let position = |point: [usize; 2]| {
                    Pos2::new(
                        (f64::from(self.origin.x) + point[0] as f64 * self.pixel_size) as f32,
                        (f64::from(self.origin.y) + point[1] as f64 * self.pixel_size) as f32,
                    )
                };
                let bounds = Rect::from_min_max(position(min), position(max));
                if bounds.is_finite() && bounds.is_positive() {
                    return Ok((bounds, threshold));
                }
                return Err("窗口边界超出有效坐标范围".into());
            }
            if threshold <= MIN_ALPHA {
                return Err("模型动画采样没有可见的 Alpha 像素，无法计算窗口边界".into());
            }
            // High coverage can miss translucent-only models. Lowering the
            // threshold preserves texture coverage; never substitute geometry.
            threshold = (threshold * 0.9375).max(MIN_ALPHA);
        }
    }
}

#[derive(Clone, Copy)]
struct Edge {
    a: f64,
    b: f64,
    c: f64,
    inclusive: bool,
}

impl Edge {
    fn new(start: [f64; 2], end: [f64; 2]) -> Self {
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        Self {
            a: -dy,
            b: dx,
            c: start[0] * end[1] - start[1] * end[0],
            // Opposite directions of a shared edge include exactly one side.
            inclusive: dy > 0.0 || (dy == 0.0 && dx < 0.0),
        }
    }

    fn at(self, point: [f64; 2]) -> f64 {
        self.a * point[0] + self.b * point[1] + self.c
    }
}

fn bilinear_alpha(image: &ColorImage, uv: [f64; 2]) -> f32 {
    let x = (uv[0] * image.size[0] as f64 - 0.5).clamp(0.0, (image.size[0] - 1) as f64);
    let y = (uv[1] * image.size[1] as f64 - 0.5).clamp(0.0, (image.size[1] - 1) as f64);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(image.size[0] - 1);
    let y1 = (y0 + 1).min(image.size[1] - 1);
    let wx = (x - x0 as f64) as f32;
    let wy = (y - y0 as f64) as f32;
    let alpha = |x: usize, y: usize| f32::from(image.pixels[y * image.size[0] + x].a()) / 255.0;
    let lower = alpha(x0, y0) * (1.0 - wx) + alpha(x1, y0) * wx;
    let upper = alpha(x0, y1) * (1.0 - wx) + alpha(x1, y1) * wx;
    lower * (1.0 - wy) + upper * wy
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;
    use std::{path::Path, time::Instant};

    fn real_model(id: &str) -> (Spine, Vec<ColorImage>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/models")
            .join(id);
        let spine = Spine::load(
            &root.join(format!("build_char_{id}.skel")),
            &root.join(format!("build_char_{id}.atlas")),
        )
        .unwrap();
        let images = spine
            .pages
            .iter()
            .map(|path| {
                let image = image::open(path).unwrap().into_rgba8();
                ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                )
            })
            .collect();
        (spine, images)
    }

    #[test]
    fn real_animation_alpha_produces_nested_finite_crops() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let (mut spine, images) = real_model(id);
            let start = Instant::now();
            let wide = calculate(
                &mut spine,
                &images,
                &CanvasSettings {
                    coverage: 0.45,
                    sampling_interval: 4,
                },
            )
            .unwrap();
            assert_eq!(wide.effective_coverage, 0.45);
            let wide = wide.rect;
            let narrow = calculate(
                &mut spine,
                &images,
                &CanvasSettings {
                    coverage: 0.95,
                    sampling_interval: 4,
                },
            )
            .unwrap();
            assert_eq!(narrow.effective_coverage, 0.95);
            let narrow = narrow.rect;
            eprintln!(
                "{id}: two Alpha crops in {:?}: {wide:?} -> {narrow:?}",
                start.elapsed()
            );
            assert!(wide.is_finite() && wide.is_positive());
            assert!(narrow.is_finite() && narrow.is_positive());
            assert!(wide.contains_rect(narrow), "{id}: {wide:?} -> {narrow:?}");
            assert_ne!(wide, narrow, "coverage must affect the real sampled Alpha");
        }
    }

    #[test]
    fn real_animations_use_the_requested_temporal_sampling() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let (mut spine, images) = real_model(id);
            let fine = CanvasSettings {
                coverage: 0.8,
                sampling_interval: 1,
            };
            let coarse = CanvasSettings {
                sampling_interval: 64,
                ..fine
            };
            let fine_plan = sampling_plan(&spine, &fine).unwrap();
            let coarse_plan = sampling_plan(&spine, &coarse).unwrap();
            let fine_count = fine_plan.iter().map(|p| p.count).sum::<usize>();
            let coarse_count = coarse_plan.iter().map(|p| p.count).sum::<usize>();
            assert!(fine_count > coarse_count);
            let start = Instant::now();
            let fine_bounds = calculate(&mut spine, &images, &fine).unwrap();
            let fine_elapsed = start.elapsed();
            let start = Instant::now();
            let coarse_bounds = calculate(&mut spine, &images, &coarse).unwrap();
            assert_eq!(fine_bounds.samples, fine_count);
            assert_eq!(coarse_bounds.samples, coarse_count);
            eprintln!(
                "{id}: interval 1={fine_elapsed:?} ({fine_count} poses), interval 64={:?} ({coarse_count} poses)",
                start.elapsed()
            );
            assert_ne!(
                fine_bounds.rect, coarse_bounds.rect,
                "{id}: temporal samples must affect the crop"
            );
        }
    }

    #[test]
    fn transparent_real_atlases_do_not_fall_back_to_vertex_bounds() {
        for id in ["4058_pepe", "1038_whitw2", "4093_frston"] {
            let (mut spine, mut images) = real_model(id);
            for image in &mut images {
                image.pixels.fill(Color32::TRANSPARENT);
            }
            let error = calculate(
                &mut spine,
                &images,
                &CanvasSettings {
                    coverage: 0.8,
                    sampling_interval: 64,
                },
            )
            .unwrap_err();
            assert!(error.contains("Alpha"), "{id}: {error}");
        }
    }

    #[test]
    fn triangles_share_an_edge_without_double_alpha_and_sample_texture_edges() {
        let mut mask = AlphaMask::new(Rect::from_min_max(Pos2::ZERO, Pos2::new(256.0, 256.0)), 1.0);
        let mesh = MeshPart {
            positions: vec![[0.0, 0.0], [256.0, 0.0], [256.0, 256.0], [0.0, 256.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 2, 3, 0],
            color: [1.0; 4],
            ..Default::default()
        };
        let image = ColorImage::new([2, 2], vec![Color32::WHITE; 4]);
        mask.draw(&mesh, &image, 0.5);
        assert!(mask.pixels.iter().all(|&alpha| (alpha - 0.5).abs() < 1e-6));
        let edge = ColorImage::new([2, 1], vec![Color32::TRANSPARENT, Color32::WHITE]);
        assert_eq!(bilinear_alpha(&edge, [0.0, 0.5]), 0.0);
        assert_eq!(bilinear_alpha(&edge, [0.5, 0.5]), 0.5);
        assert_eq!(bilinear_alpha(&edge, [1.0, 0.5]), 1.0);
    }

    #[test]
    fn translucent_crop_reports_its_lowered_coverage() {
        let mut mask = AlphaMask::new(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(256.0, 256.0)),
            0.95,
        );
        mask.pixels[128 * mask.width + 128] = 0.25;
        let (rect, effective_coverage) = mask.crop(0.95).unwrap();
        assert!(rect.is_finite() && rect.is_positive());
        assert!(effective_coverage < 0.95);
        assert!(effective_coverage >= MIN_ALPHA);
        assert!((effective_coverage * 255.0).floor() / 255.0 <= 0.25);
    }

    #[test]
    fn invalid_sampling_parameters_fail_before_modifying_pose() {
        let (spine, _) = real_model("4058_pepe");
        for coverage in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(sampling_plan(
                &spine,
                &CanvasSettings {
                    coverage,
                    sampling_interval: 4
                }
            )
            .is_err());
        }
        assert!(sampling_plan(
            &spine,
            &CanvasSettings {
                coverage: 0.8,
                sampling_interval: 0
            }
        )
        .is_err());
    }

    #[test]
    fn excessive_or_empty_animation_catalog_is_explicitly_rejected() {
        let (mut spine, _) = real_model("4058_pepe");
        spine.animations = vec!["Relax".into(); MAX_SAMPLES + 1];
        let settings = CanvasSettings {
            coverage: 0.8,
            sampling_interval: 64,
        };
        let error = sampling_plan(&spine, &settings).err().unwrap();
        assert!(error.contains("4096"));
        spine.animations.clear();
        assert!(sampling_plan(&spine, &settings).is_err());
    }
}

use arkpets::{
    spine::{MeshPart, Spine},
    CharacterModel, Direction,
};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use std::path::Path;

pub struct Renderer {
    pub spine: Spine,
    textures: Vec<TextureHandle>,
    meshes: Vec<MeshPart>,
    pub bounds: Rect,
    pub vertices: usize,
    pub triangles: usize,
}

impl Renderer {
    pub fn load(ctx: &egui::Context, model: &CharacterModel, root: &Path) -> Result<Self, String> {
        if model
            .resource_path
            .as_deref()
            .is_some_and(|path| path.contains("://"))
        {
            return Err("当前仅支持本地模型目录，请先下载资源".into());
        }
        let root = root.join(model.resource_path.as_deref().unwrap_or(""));
        let mut spine = Spine::load(&root.join(&model.skeleton), &root.join(&model.atlas))
            .map_err(|e| e.to_string())?;
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
        let mut result = Self {
            spine,
            textures: Vec::new(),
            meshes,
            bounds,
            vertices: 0,
            triangles: 0,
        };
        result.rebind_textures(ctx)?;
        result.update(0.0, 0.3)?;
        Ok(result)
    }

    /// Texture IDs belong to an egui context; retain the Spine animation when rebinding.
    pub fn rebind_textures(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let textures = self
            .spine
            .pages
            .iter()
            .map(|path| {
                let image = image::open(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .into_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Ok(ctx.load_texture(
                    path.to_string_lossy(),
                    ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                    TextureOptions::LINEAR,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.textures = textures;
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
        Vec2::new(self.bounds.width() / self.bounds.height() * height, height)
    }

    pub fn paint(&self, painter: &egui::Painter, target: Rect, direction: Direction, opacity: f32) {
        painter.extend(
            mesh_batches(
                &self.meshes,
                &self.textures,
                self.bounds,
                target,
                direction,
                opacity,
            )
            .map(egui::Shape::mesh),
        );
    }
}

fn mesh_batches<'a>(
    parts: &'a [MeshPart],
    textures: &'a [TextureHandle],
    bounds: Rect,
    target: Rect,
    direction: Direction,
    opacity: f32,
) -> impl Iterator<Item = egui::Mesh> + 'a {
    let scale = target.height() / bounds.height();
    // Only adjacent slots may be merged: sorting by texture changes occlusion.
    // Additive blending is encoded in each vertex's premultiplied color.
    parts.chunk_by(|a, b| a.page == b.page).map(move |batch| {
        let mut mesh = egui::Mesh::with_texture(textures[batch[0].page].id());
        mesh.vertices
            .reserve_exact(batch.iter().map(|part| part.positions.len()).sum());
        mesh.indices
            .reserve_exact(batch.iter().map(|part| part.indices.len()).sum());
        for part in batch {
            let index_offset = mesh.vertices.len() as u32;
            let [r, g, b, a] = part.color;
            let alpha = (a * opacity * 255.0).clamp(0.0, 255.0) as u8;
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
                            target.left()
                                + if direction == Direction::Left {
                                    target.width() - x
                                } else {
                                    x
                                },
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

    #[test]
    fn rebinding_uploads_textures_to_the_new_context_without_resetting_animation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let catalog = arkpets::ModelCatalog::from_json_file(root.join("models.json")).unwrap();
        let original = egui::Context::default();
        let mut renderer = Renderer::load(&original, &catalog.models[0], &root).unwrap();
        renderer.update(0.25, 0.3).unwrap();
        let positions: Vec<_> = renderer
            .meshes
            .iter()
            .map(|part| part.positions.clone())
            .collect();
        // Consume the initial uploads, as a native painter would have done.
        original
            .run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();

        let fresh = egui::Context::default();
        let _reserved = fresh.load_texture(
            "reserved",
            ColorImage::from_rgba_unmultiplied([1, 1], &[255; 4]),
            TextureOptions::LINEAR,
        );
        fresh
            .run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();
        renderer.rebind_textures(&fresh).unwrap();
        let output = fresh.run_ui(Default::default(), |ui| {
            renderer.paint(
                ui.painter(),
                Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)),
                Direction::Right,
                1.0,
            );
        });
        for texture in &renderer.textures {
            let deltas = output
                .textures_delta
                .set
                .get(&texture.id())
                .expect("every atlas must be uploaded to the new painter");
            assert_eq!(deltas.len(), 1);
            let delta = &deltas[0];
            assert_eq!(delta.image.size(), texture.size());
            assert!(
                delta.pos.is_none(),
                "a new painter needs a full texture upload"
            );
        }
        assert_eq!(
            renderer
                .meshes
                .iter()
                .map(|part| part.positions.clone())
                .collect::<Vec<_>>(),
            positions
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn batches_preserve_slot_order_indices_and_additive_colors() {
        let ctx = egui::Context::default();
        let textures: Vec<_> = (0..2)
            .map(|page| {
                ctx.load_texture(
                    format!("page-{page}"),
                    ColorImage::from_rgba_unmultiplied([1, 1], &[255; 4]),
                    TextureOptions::LINEAR,
                )
            })
            .collect();
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
        let meshes: Vec<_> =
            mesh_batches(&parts, &textures, bounds, target, Direction::Right, 1.0).collect();
        assert_eq!(meshes.len(), 3);
        assert_eq!(
            meshes
                .iter()
                .map(|mesh| mesh.texture_id)
                .collect::<Vec<_>>(),
            [textures[0].id(), textures[1].id(), textures[0].id()]
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

        let mirrored: Vec<_> =
            mesh_batches(&parts, &textures, bounds, target, Direction::Left, 0.5).collect();
        assert_eq!(mirrored[0].vertices[3].pos, Pos2::new(180.0, 200.0));
        assert_eq!(mirrored[0].vertices[0].color.a(), 63);
        assert_eq!(mirrored[0].vertices[3].color.a(), 0);
    }
}

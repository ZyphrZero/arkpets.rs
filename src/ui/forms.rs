//! One logical character with an active renderer and bounded, inactive form resources.
use super::renderer::Renderer;
use arkpets::{config::CanvasSettings, model::ModelForm, CharacterModel, ModelCatalog};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Base,
    Battle,
}

impl Mode {
    // Auto is retained only for reading old preferences and diagnostic input.
    pub fn resolve(self, preferred: ModelForm) -> ModelForm {
        match self {
            Self::Auto => preferred,
            Self::Base => ModelForm::Base,
            Self::Battle => ModelForm::Battle,
        }
    }
}

pub fn label(form: ModelForm) -> &'static str {
    match form {
        ModelForm::Base => "基建",
        ModelForm::Battle => "战斗",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    pub character_id: String,
    pub skin_id: String,
    pub mode: Mode,
    pub preferred: ModelForm,
}

impl Preference {
    pub fn matches(&self, model: &CharacterModel) -> bool {
        model.character_id.as_deref() == Some(&self.character_id)
            && model.skin_id.as_deref() == Some(&self.skin_id)
    }
}

struct Entry {
    index: usize,
    form: Option<ModelForm>,
}

pub struct Forms {
    pub active: usize,
    entries: Vec<Entry>,
    inactive: HashMap<usize, Renderer>,
}

impl Forms {
    pub fn load(
        catalog: &ModelCatalog,
        index: usize,
        root: &Path,
        canvas: CanvasSettings,
        saved: &[Preference],
    ) -> Result<(Self, Renderer), String> {
        let model = &catalog.models[index];
        let preference = saved.iter().find(|preference| preference.matches(model));
        let preferred = model.form.unwrap_or(ModelForm::Base);
        let selected = preference.map_or(preferred, |value| value.mode.resolve(value.preferred));
        let mut result = Self {
            active: index,
            entries: Vec::new(),
            inactive: HashMap::new(),
        };
        for index in catalog
            .form_indices(index)
            .map_err(|error| error.to_string())?
        {
            let renderer = Renderer::load(&catalog.models[index], root, canvas)
                .map_err(|error| format!("载入 {} 失败: {error}", catalog.models[index].name))?;
            result.entries.push(Entry {
                index,
                form: catalog.models[index].form,
            });
            result.inactive.insert(index, renderer);
        }
        if model.form.is_some() {
            result.active = result.index_for(selected)?;
        }
        let renderer = result
            .inactive
            .remove(&result.active)
            .expect("loaded active form");
        Ok((result, renderer))
    }

    pub fn has(&self, form: ModelForm) -> bool {
        self.entries.iter().any(|entry| entry.form == Some(form))
    }

    pub fn index_for(&self, form: ModelForm) -> Result<usize, String> {
        self.entries
            .iter()
            .find(|entry| entry.form == Some(form))
            .map(|entry| entry.index)
            .ok_or_else(|| format!("当前角色没有{}形态", label(form)))
    }

    pub fn active_form(&self) -> Option<ModelForm> {
        self.entries
            .iter()
            .find(|entry| entry.index == self.active)
            .and_then(|entry| entry.form)
    }

    pub fn prepare(&mut self, index: usize, canvas: CanvasSettings) -> Result<&Renderer, String> {
        let renderer = self
            .inactive
            .get_mut(&index)
            .ok_or("切换形态的资源未载入")?;
        if renderer.canvas_settings != canvas {
            renderer.recalculate_bounds(canvas)?;
        }
        // Form changes are explicit resource switches, never animation-name routing.
        let animation = arkpets::animation::initial_animation(&renderer.spine.animations)
            .unwrap()
            .to_owned();
        renderer
            .spine
            .play_with_mix(
                &animation,
                arkpets::animation::animation_loops(&animation),
                0.0,
            )
            .map_err(|error| error.to_string())?;
        renderer.update(0.0, 0.0)?;
        Ok(renderer)
    }

    pub fn activate(&mut self, index: usize, renderer: &mut Renderer) {
        let next = self.inactive.remove(&index).expect("prepared form");
        let old = std::mem::replace(renderer, next);
        self.inactive.insert(self.active, old);
        self.active = index;
    }
}

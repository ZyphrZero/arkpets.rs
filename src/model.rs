use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelForm {
    Base,
    Battle,
}

/// A Spine character and the files required by a renderer to load it.
///
/// The first six fields mirror ArkPets-Web's `CharacterModel`. The remaining
/// metadata fields accept the richer `ModelItem` records from the original
/// desktop model dataset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CharacterModel {
    pub id: String,
    pub name: String,
    pub character_id: Option<String>,
    pub skin_id: Option<String>,
    pub form: Option<ModelForm>,
    pub skeleton: String,
    pub atlas: String,
    pub texture: String,
    pub resource_path: Option<String>,
    pub animations: Vec<String>,
    pub asset_id: Option<String>,
    #[serde(rename = "type")]
    pub model_type: Option<String>,
    pub style: Option<String>,
    pub appellation: Option<String>,
    pub skin_group_id: Option<String>,
    pub skin_group_name: Option<String>,
    pub sort_tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_list: Option<Value>,
    pub pinyin_full: Option<String>,
    pub pinyin_initials: Option<String>,
}

impl CharacterModel {
    pub fn same_character(&self, other: &Self) -> bool {
        match (
            &self.character_id,
            &self.skin_id,
            &other.character_id,
            &other.skin_id,
        ) {
            (Some(a), Some(skin_a), Some(b), Some(skin_b)) => a == b && skin_a == skin_b,
            (None, None, None, None) => self.id == other.id,
            _ => false,
        }
    }

    pub fn resource_names(&self) -> Vec<&str> {
        [&self.skeleton, &self.atlas, &self.texture]
            .into_iter()
            .filter(|value| !value.is_empty())
            .map(String::as_str)
            .collect()
    }

    pub fn resource_paths(&self, base_dir: impl AsRef<Path>) -> Vec<PathBuf> {
        let base_dir = base_dir
            .as_ref()
            .join(self.resource_path.as_deref().unwrap_or(""));
        self.resource_names()
            .into_iter()
            .map(|name| base_dir.join(name))
            .collect()
    }

    pub fn validate(&self, base_dir: Option<impl AsRef<Path>>) -> Result<(), ModelError> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(ModelError::Invalid(
                self.id.clone(),
                "id and name are required",
            ));
        }
        if self.resource_names().len() != 3 {
            return Err(ModelError::Invalid(
                self.id.clone(),
                "skeleton, atlas, and texture are required",
            ));
        }
        if let Some(base_dir) = base_dir {
            for path in self.resource_paths(base_dir) {
                if !path.is_file() {
                    return Err(ModelError::MissingAsset(path));
                }
            }
        }
        Ok(())
    }

    fn from_dataset_item(id: &str, value: Value) -> Result<Self, ModelError> {
        let mut model: CharacterModel = serde_json::from_value(value)?;
        if model.id.is_empty() {
            model.id = id.to_string();
        }
        if let Some(asset_list) = model.asset_list.clone() {
            model.skeleton = model.skeleton.or_from_asset_list(&asset_list, ".skel");
            model.atlas = model.atlas.or_from_asset_list(&asset_list, ".atlas");
            model.texture = model.texture.or_from_asset_list(&asset_list, ".png");
        }
        if model.asset_list.is_none() {
            if let Some(id) = &model.asset_id {
                if model.skeleton.is_empty() {
                    model.skeleton = format!("{id}.skel");
                }
                if model.atlas.is_empty() {
                    model.atlas = format!("{id}.atlas");
                }
                if model.texture.is_empty() {
                    model.texture = format!("{id}.png");
                }
            }
        }
        Ok(model)
    }
}

trait EmptyStringExt {
    fn or_from_asset_list(self, list: &Value, extension: &str) -> String;
}

impl EmptyStringExt for String {
    fn or_from_asset_list(self, list: &Value, extension: &str) -> String {
        if !self.is_empty() {
            return self;
        }
        if let Some(value) = list.get(extension) {
            if let Some(name) = value.as_str() {
                return name.to_string();
            }
            if let Some(items) = value.as_array() {
                if let Some(name) = items.iter().find_map(Value::as_str) {
                    return name.to_string();
                }
            }
        }
        self
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("failed to read model file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid model JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model {0} is invalid: {1}")]
    Invalid(String, &'static str),
    #[error("model asset is missing: {0}")]
    MissingAsset(PathBuf),
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<CharacterModel>,
}

impl ModelCatalog {
    pub fn form_indices(&self, index: usize) -> Result<Vec<usize>, ModelError> {
        let selected = &self.models[index];
        if selected.character_id.is_none() && selected.skin_id.is_none() && selected.form.is_none()
        {
            return Ok(vec![index]);
        }
        if selected
            .character_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
            || selected
                .skin_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
            || selected.form.is_none()
        {
            return Err(ModelError::Invalid(
                selected.id.clone(),
                "characterId, skinId and form must be provided together and nonempty",
            ));
        }
        let mut indices = Vec::new();
        for (index, model) in self
            .models
            .iter()
            .enumerate()
            .filter(|(_, model)| selected.same_character(model))
        {
            if model.form.is_some()
                && indices
                    .iter()
                    .any(|&i: &usize| self.models[i].form == model.form)
            {
                return Err(ModelError::Invalid(
                    selected.id.clone(),
                    "duplicate form for the same character and skin",
                ));
            }
            indices.push(index);
        }
        Ok(indices)
    }

    pub fn representative(&self, index: usize) -> usize {
        if self.models[index].character_id.is_none() {
            return index;
        }
        self.models
            .iter()
            .position(|model| {
                self.models[index].same_character(model) && model.form == Some(ModelForm::Base)
            })
            .unwrap_or(index)
    }

    pub fn character_indices(&self) -> Vec<usize> {
        let mut result = Vec::new();
        for index in 0..self.models.len() {
            let representative = self.representative(index);
            if !result.contains(&representative) {
                result.push(representative);
            }
        }
        result
    }

    fn checked(models: Vec<CharacterModel>) -> Result<Self, ModelError> {
        let catalog = Self::new(models);
        for index in 0..catalog.models.len() {
            catalog.form_indices(index)?;
        }
        Ok(catalog)
    }

    pub fn new(models: Vec<CharacterModel>) -> Self {
        Self { models }
    }

    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let value: Value = serde_json::from_str(text)?;
        let models = match value {
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, item)| CharacterModel::from_dataset_item(&index.to_string(), item))
                .collect::<Result<Vec<_>, _>>()?,
            Value::Object(mut object) => {
                if object.contains_key("skeleton") {
                    return Self::checked(vec![CharacterModel::from_dataset_item(
                        "",
                        Value::Object(object),
                    )?]);
                }
                if let Some(models) = object.remove("models") {
                    return Self::from_json(&serde_json::to_string(&models)?);
                }
                let storage = object.remove("storageDirectory");
                let data = object.remove("data").unwrap_or(Value::Object(object));
                let Value::Object(items) = data else {
                    return Err(ModelError::Invalid(
                        "dataset".into(),
                        "data must be an object or an array",
                    ));
                };
                items
                    .into_iter()
                    .map(|(id, item)| {
                        let mut model = CharacterModel::from_dataset_item(&id, item)?;
                        if let Some(storage) = &storage {
                            let kind = model.model_type.as_deref().ok_or_else(|| {
                                ModelError::Invalid(id.clone(), "type is required")
                            })?;
                            let directory =
                                storage.get(kind).and_then(Value::as_str).ok_or_else(|| {
                                    ModelError::Invalid(
                                        id.clone(),
                                        "type missing from storageDirectory",
                                    )
                                })?;
                            model.resource_path =
                                Some(Path::new(directory).join(id).to_string_lossy().into_owned());
                        }
                        Ok::<_, ModelError>(model)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            _ => {
                return Err(ModelError::Invalid(
                    "dataset".into(),
                    "root must be an object or array",
                ))
            }
        };
        Self::checked(models)
    }

    pub fn from_json_file(path: impl AsRef<Path>) -> Result<Self, ModelError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| ModelError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json(&text)
    }

    pub fn search(&self, query: &str) -> Vec<&CharacterModel> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return self.models.iter().collect();
        }
        self.models
            .iter()
            .filter(|model| {
                [
                    model.id.as_str(),
                    model.name.as_str(),
                    model.style.as_deref().unwrap_or(""),
                    model.appellation.as_deref().unwrap_or(""),
                    model.skin_group_name.as_deref().unwrap_or(""),
                    model.pinyin_full.as_deref().unwrap_or(""),
                    model.pinyin_initials.as_deref().unwrap_or(""),
                ]
                .iter()
                .any(|field| field.to_ascii_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn by_type(&self, model_type: &str) -> Vec<&CharacterModel> {
        self.models
            .iter()
            .filter(|model| model.model_type.as_deref() == Some(model_type))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_paired_only_by_explicit_character_and_skin_metadata() {
        let json = r#"[
            {"id":"a","name":"A","characterId":"texas","skinId":"default","form":"base"},
            {"id":"b","name":"B","characterId":"texas","skinId":"default","form":"battle"},
            {"id":"c","name":"A（战斗）","characterId":"texas","skinId":"skin2","form":"battle"},
            {"id":"d","name":"A（战斗）"}
        ]"#;
        let catalog = ModelCatalog::from_json(json).unwrap();
        assert_eq!(catalog.form_indices(0).unwrap(), [0, 1]);
        assert_eq!(catalog.form_indices(2).unwrap(), [2]);
        assert_eq!(catalog.form_indices(3).unwrap(), [3]);
        assert_eq!(catalog.character_indices(), [0, 2, 3]);
        assert!(ModelCatalog::from_json(r#"[{"id":"x","characterId":"x"}]"#).is_err());
        assert!(
            ModelCatalog::from_json(&json.replace("\"form\":\"base\"", "\"form\":\"battle\""))
                .is_err()
        );
    }

    #[test]
    fn parses_web_manifest_and_searches_metadata() {
        let catalog = ModelCatalog::from_json(r#"[{"id":"pepe","name":"佩佩","skeleton":"a.skel","atlas":"a.atlas","texture":"a.png","pinyinFull":"PEIPEI"}]"#).unwrap();
        assert_eq!(catalog.search("peipei").len(), 1);
        assert_eq!(
            catalog.models[0].resource_names(),
            vec!["a.skel", "a.atlas", "a.png"]
        );
    }

    #[test]
    fn parses_original_dataset_asset_list() {
        let catalog = ModelCatalog::from_json(r#"{"data":{"pepe":{"name":"佩佩","assetList":{".skel":["build.skel"],".atlas":["build.atlas"],".png":["build.png"]}}}}"#).unwrap();
        let model = &catalog.models[0];
        assert_eq!(model.id, "pepe");
        assert_eq!(model.skeleton, "build.skel");
    }

    #[test]
    fn dataset_storage_and_legacy_asset_id_resolve_relative_to_manifest() {
        let catalog = ModelCatalog::from_json(r#"{"storageDirectory":{"operator":"models"},"data":{"test#1":{"name":"Test","type":"operator","assetId":"build"}}}"#).unwrap();
        let files = catalog.models[0].resource_paths("assets");
        assert_eq!(files[0], Path::new("assets/models/test#1/build.skel"));
    }

    #[test]
    fn single_model_and_catalog_round_trip() {
        let catalog = ModelCatalog::from_json(r#"{"id":"test","name":"Test","skeleton":"a.skel","atlas":"a.atlas","texture":"a.png"}"#).unwrap();
        let restored = ModelCatalog::from_json(&serde_json::to_string(&catalog).unwrap()).unwrap();
        assert_eq!(restored.models, catalog.models);
    }
}

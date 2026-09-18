use serde::{Deserialize, Serialize};
use std::{env, ffi::OsString, fs, io, path::Path, path::PathBuf, str::FromStr};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererBackend {
    #[cfg_attr(windows, default)]
    Dx12,
    OpenGl,
    #[cfg_attr(target_os = "macos", default)]
    Metal,
    #[cfg_attr(not(any(windows, target_os = "macos")), default)]
    Vulkan,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    renderer: RendererBackend,
}

impl RendererBackend {
    pub fn available() -> &'static [Self] {
        #[cfg(windows)]
        {
            &[Self::Dx12, Self::OpenGl]
        }
        #[cfg(target_os = "macos")]
        {
            &[Self::Metal]
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            &[Self::Vulkan]
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dx12 => "dx12",
            Self::OpenGl => "opengl",
            Self::Metal => "metal",
            Self::Vulkan => "vulkan",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dx12 => "DirectX 12",
            Self::OpenGl => "OpenGL",
            Self::Metal => "Metal",
            Self::Vulkan => "Vulkan",
        }
    }

    pub fn validate(self) -> Result<Self, String> {
        if Self::available().contains(&self) {
            Ok(self)
        } else {
            Err(format!(
                "Renderer {} is unavailable on {}; available: {}",
                self.as_str(),
                env::consts::OS,
                available_names()
            ))
        }
    }

    pub fn renderer(self) -> Result<eframe::Renderer, String> {
        self.validate()?;
        #[cfg(windows)]
        if self == Self::OpenGl {
            return Ok(eframe::Renderer::Glow);
        }
        Ok(eframe::Renderer::Wgpu)
    }

    pub fn wgpu_backend(self) -> Option<eframe::wgpu::Backends> {
        match self {
            Self::Dx12 => Some(eframe::wgpu::Backends::DX12),
            Self::OpenGl => None,
            Self::Metal => Some(eframe::wgpu::Backends::METAL),
            Self::Vulkan => Some(eframe::wgpu::Backends::VULKAN),
        }
    }

    /// Precedence: command line, environment, bootstrap file, platform default.
    pub fn requested() -> Result<Self, String> {
        if let Some(backend) = parse_arguments(env::args_os().skip(1))? {
            return Ok(backend);
        }
        match env::var("ARKPETS_RENDERER") {
            Ok(value) => return value.parse(),
            Err(env::VarError::NotPresent) => {}
            Err(error) => return Err(format!("ARKPETS_RENDERER: {error}")),
        }
        read_config(&config_path()?)
    }

    /// Persist the backend preference independently of the current GPU resources.
    pub fn save(self) -> Result<(), String> {
        write_config(&config_path()?, self)
    }
}

impl FromStr for RendererBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let backend = match value {
            "dx12" => Self::Dx12,
            "opengl" => Self::OpenGl,
            "metal" => Self::Metal,
            "vulkan" => Self::Vulkan,
            _ => {
                return Err(format!(
                    "Unknown renderer {value:?}; available: {}",
                    available_names()
                ));
            }
        };
        backend.validate()
    }
}

fn available_names() -> String {
    RendererBackend::available()
        .iter()
        .map(|backend| backend.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_arguments(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<Option<RendererBackend>, String> {
    let mut selected = None;
    while let Some(argument) = arguments.next() {
        let argument = argument
            .into_string()
            .map_err(|_| "Renderer arguments must be valid Unicode".to_owned())?;
        let value = if argument == "--renderer" {
            arguments
                .next()
                .ok_or_else(|| "--renderer requires a value".to_owned())?
                .into_string()
                .map_err(|_| "Renderer name must be valid Unicode".to_owned())?
        } else if let Some(value) = argument.strip_prefix("--renderer=") {
            value.to_owned()
        } else {
            return Err(format!(
                "Unknown argument {argument:?}; expected --renderer <backend>"
            ));
        };
        if selected.is_some() {
            return Err("--renderer may only be specified once".into());
        }
        selected = Some(value.parse()?);
    }
    Ok(selected)
}

fn config_path() -> Result<PathBuf, String> {
    // Diagnostic runs must not read or overwrite the user's bootstrap settings.
    let directory = env::var_os("ARKPETS_CAPTURE_DIR")
        .or_else(|| env::var_os("ARKPETS_PROFILE_DIR"))
        .map(PathBuf::from)
        .or_else(|| eframe::storage_dir("ArkPets"))
        .ok_or_else(|| "Cannot locate the ArkPets user data directory".to_owned())?;
    Ok(directory.join("bootstrap.json"))
}

fn read_config(path: &Path) -> Result<RendererBackend, String> {
    let json = match fs::read(path) {
        Ok(json) => json,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RendererBackend::default())
        }
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    let bootstrap: Bootstrap = serde_json::from_slice(&json)
        .map_err(|error| format!("Invalid {}: {error}", path.display()))?;
    bootstrap.renderer.validate()
}

fn write_config(path: &Path, renderer: RendererBackend) -> Result<(), String> {
    renderer.validate()?;
    let json = serde_json::to_vec_pretty(&Bootstrap { renderer })
        .map_err(|error| format!("Cannot encode renderer settings: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "Renderer settings path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, json)
        .map_err(|error| format!("Cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Cannot replace {}: {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_renderer_values_are_validated_without_fallback() {
        for &backend in RendererBackend::available() {
            assert_eq!(backend.as_str().parse::<RendererBackend>(), Ok(backend));
            assert!(backend.renderer().is_ok());
        }
        for value in ["", "auto", "unknown"] {
            assert!(value.parse::<RendererBackend>().is_err());
        }
        for backend in [
            RendererBackend::Dx12,
            RendererBackend::OpenGl,
            RendererBackend::Metal,
            RendererBackend::Vulkan,
        ] {
            if !RendererBackend::available().contains(&backend) {
                assert!(backend.as_str().parse::<RendererBackend>().is_err());
                assert!(backend.renderer().is_err());
            }
        }
    }

    #[test]
    fn command_line_requires_one_complete_renderer_argument() {
        let name = RendererBackend::default().as_str();
        let parse =
            |args: &[&str]| parse_arguments(args.iter().map(|argument| OsString::from(*argument)));
        assert_eq!(parse(&[]), Ok(None));
        assert_eq!(
            parse(&["--renderer", name]),
            Ok(Some(RendererBackend::default()))
        );
        assert_eq!(
            parse(&[&format!("--renderer={name}")]),
            Ok(Some(RendererBackend::default()))
        );
        assert!(parse(&["--renderer"]).is_err());
        assert!(parse(&["--renderer", name, "--renderer", name]).is_err());
        assert!(parse(&["--renderer", "auto"]).is_err());
        assert!(parse(&["--unknown"]).is_err());
    }

    #[test]
    fn bootstrap_can_be_replaced_and_invalid_saved_settings_are_reported() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "arkpets-backend-{}-{unique}.json",
            std::process::id()
        ));
        assert_eq!(read_config(&path), Ok(RendererBackend::default()));
        for &backend in RendererBackend::available() {
            write_config(&path, backend).unwrap();
            assert_eq!(read_config(&path), Ok(backend));
        }
        fs::write(&path, br#"{"renderer":"unknown"}"#).unwrap();
        assert!(read_config(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}

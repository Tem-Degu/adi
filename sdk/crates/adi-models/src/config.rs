//! Which file serves which seam.
//!
//! YAML rather than TOML, because every other data file in this SDK is YAML
//! and a developer editing a declaration should not change format to edit a
//! configuration.
//!
//! A file extension does not say what context length to use or what sample
//! rate a voice was trained at, so those are stated here rather than guessed
//! from the file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::Seam;
use crate::ModelError;

/// How a model is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// A GGUF model through llama.cpp.
    Llama,
    /// An ONNX model through ONNX Runtime.
    Onnx,
    /// An external program, spoken to over its standard input and output.
    ///
    /// This is what makes a model we have never heard of usable without any
    /// change here: a script, another runtime, a service reached by something
    /// else. See `crate::process`.
    Process,
    /// Answers read from a file. For testing a path without a model in it.
    Scripted,
    /// Nothing. The seam reports itself missing, which is a supported state.
    None,
}

/// One seam's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeamConfig {
    pub backend: Backend,

    /// The model file, relative to the models folder.
    #[serde(default)]
    pub model: Option<PathBuf>,

    /// For `Process`: the program to run, and its arguments.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,

    /// Upper bound on the prompt this model is given, in tokens.
    #[serde(default)]
    pub context_tokens: Option<usize>,

    /// The sample rate a speech backend requires. ADI-P 9.4.2 refuses audio at
    /// any other rate rather than resampling it silently.
    #[serde(default)]
    pub sample_rate: Option<u32>,

    /// Below this, a transcript is treated as not heard and the machine asks.
    #[serde(default)]
    pub min_confidence: Option<f64>,

    /// Anything a particular backend needs and nothing else does.
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

impl SeamConfig {
    pub fn none() -> Self {
        Self {
            backend: Backend::None,
            model: None,
            command: None,
            args: Vec::new(),
            context_tokens: None,
            sample_rate: None,
            min_confidence: None,
            options: BTreeMap::new(),
        }
    }
}

/// The whole configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// Where model files live. Relative paths resolve against the config file.
    #[serde(default)]
    pub root: Option<PathBuf>,
    #[serde(default)]
    pub seams: BTreeMap<String, SeamConfig>,
}

impl ModelsConfig {
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ModelError::Io(path.display().to_string(), e.to_string()))?;
        let mut config: Self = serde_yaml::from_str(&text)
            .map_err(|e| ModelError::Malformed(path.display().to_string(), e.to_string()))?;

        // Resolve the root against the config's own location, so a config can
        // be moved with its models and still work.
        if let Some(parent) = path.parent() {
            let root = config.root.unwrap_or_else(|| PathBuf::from("."));
            config.root = Some(if root.is_absolute() {
                root
            } else {
                parent.join(root)
            });
        }

        Ok(config)
    }

    pub fn seam(&self, seam: Seam) -> SeamConfig {
        self.seams
            .get(seam.as_str())
            .cloned()
            .unwrap_or_else(SeamConfig::none)
    }

    /// The path of a seam's model file, if it names one.
    ///
    /// Normalised, because a root of `.` joined to a relative file produces a
    /// path that is correct and unreadable, and this path is printed to people
    /// looking for a missing file.
    pub fn model_path(&self, seam: Seam) -> Option<PathBuf> {
        let config = self.seam(seam);
        let model = config.model?;

        let joined = match &self.root {
            Some(root) if model.is_relative() => root.join(model),
            _ => model,
        };

        Some(normalise(&joined))
    }
}

/// Drop `.` components and apply the platform's separator, so a path shown to
/// a person reads the way they would have written it.
fn normalise(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, contents: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("adi-models-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_seam_can_name_a_model_file() {
        let path = write(
            "models.yaml",
            "root: weights\nseams:\n  interpreter:\n    backend: llama\n    model: qwen.gguf\n    context_tokens: 2048\n",
        );
        let config = ModelsConfig::load(&path).expect("loads");

        let interpreter = config.seam(Seam::Interpreter);
        assert_eq!(interpreter.backend, Backend::Llama);
        assert_eq!(interpreter.context_tokens, Some(2048));

        let model = config.model_path(Seam::Interpreter).expect("a path");
        assert!(model.ends_with("qwen.gguf"), "got {}", model.display());
        assert!(
            model.to_string_lossy().contains("weights"),
            "resolved against the configured root: {}",
            model.display()
        );
        assert!(
            !model.to_string_lossy().contains(&format!(".{}", std::path::MAIN_SEPARATOR)),
            "no bare current-directory components: {}",
            model.display()
        );
    }

    /// A seam nobody configured is not an error. ADI-P 9.4.2 permits an engine
    /// to ship without any model, provided it names what is missing.
    #[test]
    fn an_absent_seam_is_none_rather_than_a_failure() {
        let path = write("empty.yaml", "seams: {}\n");
        let config = ModelsConfig::load(&path).expect("loads");

        assert_eq!(config.seam(Seam::Speaker).backend, Backend::None);
        assert!(config.model_path(Seam::Speaker).is_none());
    }

    #[test]
    fn a_process_backend_names_a_command_rather_than_a_file() {
        let path = write(
            "process.yaml",
            "seams:\n  interpreter:\n    backend: process\n    command: ./my-model\n    args: [\"--json\"]\n",
        );
        let config = ModelsConfig::load(&path).expect("loads");
        let interpreter = config.seam(Seam::Interpreter);

        assert_eq!(interpreter.backend, Backend::Process);
        assert_eq!(interpreter.command.as_deref(), Some("./my-model"));
        assert_eq!(interpreter.args, vec!["--json"]);
        assert!(interpreter.model.is_none(), "the program owns its weights");
    }

    #[test]
    fn a_malformed_config_says_which_file_and_why() {
        let path = write("bad.yaml", "seams:\n  interpreter:\n    backend: nonsense\n");
        let err = ModelsConfig::load(&path).expect_err("should refuse");
        let text = format!("{err}");
        assert!(text.contains("bad.yaml"), "names the file: {text}");
    }

    #[test]
    fn an_absolute_model_path_is_left_alone() {
        let absolute = if cfg!(windows) { "C:/models/x.gguf" } else { "/models/x.gguf" };
        let path = write(
            "abs.yaml",
            &format!("root: weights\nseams:\n  interpreter:\n    backend: llama\n    model: {absolute}\n"),
        );
        let config = ModelsConfig::load(&path).expect("loads");
        let model = config.model_path(Seam::Interpreter).unwrap();
        assert!(!model.to_string_lossy().contains("weights"), "not joined to root");
    }
}

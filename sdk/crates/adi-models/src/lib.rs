//! Model backends: what is configured, what is loaded, and how a model that
//! is not ours is reached.
//!
//! `adi-language` owns the four seams as traits. This crate owns everything
//! around them: which file serves which seam, whether we can say afterwards
//! which model produced an answer, and the backend that lets an external
//! program serve a seam without any code here knowing about it.
//!
//! The SDK ships no weights. See `manifest` for why, and for what replaces a
//! downloader.
//!
//! Nothing here is domain specific. A model that interprets a request about
//! coffee and one that interprets a request about a greenhouse differ in their
//! weights and in the vocabulary handed to them, not in anything this crate
//! does.

pub mod chooser;
pub mod config;
pub mod manifest;
pub mod parse;
pub mod process;
pub mod schema;
pub mod speech;
pub mod scripted;

pub use chooser::ProcessChooser;
pub use config::{Backend, ModelsConfig, SeamConfig};
pub use parse::understood;
pub use manifest::{hash_file, identify, Manifest, Provenance, Published, Seam};
pub use process::{ProcessInterpreter, Surroundings};
pub use speech::{ProcessSpeaker, ProcessTranscriber};
pub use schema::{OUTPUT_INSTRUCTION, UNDERSTOOD_SCHEMA};
pub use scripted::ScriptedInterpreter;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("{0}: {1}")]
    Io(String, String),
    #[error("{0} is malformed: {1}")]
    Malformed(String, String),
    #[error("backend: {0}")]
    Backend(String),
    #[error("no model is configured for the {0} seam")]
    NotConfigured(&'static str),
}

/// What a seam reports about itself, for `adi models check` and for records.
#[derive(Debug, Clone)]
pub struct SeamStatus {
    pub seam: Seam,
    pub backend: Backend,
    /// Absent when the seam is unconfigured or the file is missing.
    pub provenance: Option<Provenance>,
    /// Why the seam is not usable, where it is not.
    pub problem: Option<String>,
}

impl SeamStatus {
    pub fn ready(&self) -> bool {
        self.problem.is_none() && self.backend != Backend::None
    }
}

/// Report on every seam: what is configured, what is present, and what each
/// file turned out to be.
///
/// This is what makes a setup page unnecessary. A page describes what should
/// be there; this reads what is.
pub fn survey(config: &ModelsConfig, manifest: &Manifest) -> Vec<SeamStatus> {
    [
        Seam::Transcriber,
        Seam::Interpreter,
        Seam::Explainer,
        Seam::Speaker,
    ]
    .into_iter()
    .map(|seam| survey_one(seam, config, manifest))
    .collect()
}

fn survey_one(seam: Seam, config: &ModelsConfig, manifest: &Manifest) -> SeamStatus {
    let seam_config = config.seam(seam);

    match seam_config.backend {
        Backend::None => SeamStatus {
            seam,
            backend: Backend::None,
            provenance: None,
            problem: None,
        },

        // A program supplies its own weights, so there is no file here to
        // identify. Its identity is the command, which the config records.
        Backend::Process => SeamStatus {
            seam,
            backend: Backend::Process,
            provenance: None,
            problem: seam_config
                .command
                .is_none()
                .then(|| "the process backend names no command".to_string()),
        },

        // Named in the configuration format, with nothing behind it. Report
        // that rather than inspecting the file, because a present file is not
        // a usable seam when no code can open it.
        backend if !backend.implemented() => SeamStatus {
            seam,
            backend,
            provenance: None,
            problem: Some(format!(
                "the '{}' backend is not implemented; use 'process' to serve this seam with an external program",
                backend.as_str()
            )),
        },

        _ => {
            let Some(path) = config.model_path(seam) else {
                return SeamStatus {
                    seam,
                    backend: seam_config.backend,
                    provenance: None,
                    problem: Some("no model file is named".into()),
                };
            };

            if !path.exists() {
                return SeamStatus {
                    seam,
                    backend: seam_config.backend,
                    provenance: None,
                    problem: Some(format!("{} is not present", path.display())),
                };
            }

            match identify(seam, &path, manifest) {
                Ok(provenance) => SeamStatus {
                    seam,
                    backend: seam_config.backend,
                    provenance: Some(provenance),
                    problem: None,
                },
                Err(e) => SeamStatus {
                    seam,
                    backend: seam_config.backend,
                    provenance: None,
                    problem: Some(e.to_string()),
                },
            }
        }
    }
}

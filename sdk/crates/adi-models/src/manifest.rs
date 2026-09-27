//! Which model is loaded, and whether we can say so afterwards.
//!
//! The SDK ships no weights. A developer obtains them from whoever publishes
//! them, under that party's licence, and places them in a folder. This is the
//! published list of what is recommended, and the check applied to whatever is
//! actually found.
//!
//! **Verification is for provenance, not gatekeeping.** A file matching no
//! published hash still loads. Refusing one would defeat the point of the
//! folder, which is that a manufacturer may use a model we have never heard
//! of. What is not permitted is loading a model and being unable to say later
//! which one it was.
//!
//! ADI-P 9.3 requirement 5 obliges the engine to retain enough to reconstruct a
//! decision. Where a model contributed, its identity is part of that. A
//! manufacturer who changes the interpreter should find that every record from
//! that point states it.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ModelError;

/// Which seam a model serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seam {
    Transcriber,
    Interpreter,
    Explainer,
    Speaker,
}

impl Seam {
    pub fn as_str(self) -> &'static str {
        match self {
            Seam::Transcriber => "transcriber",
            Seam::Interpreter => "interpreter",
            Seam::Explainer => "explainer",
            Seam::Speaker => "speaker",
        }
    }
}

/// One model the SDK recommends and can recognise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Published {
    /// Stable identifier used in records, for example `qwen3-1.7b-q4_k_m`.
    pub id: String,
    pub seam: Seam,
    /// The file name expected in the folder.
    pub file: String,
    /// Lowercase hex SHA-256 of that exact file.
    pub sha256: String,
    pub bytes: u64,
    /// Where to obtain it. Stated so a developer need not search.
    pub source: String,
    /// The licence the weights are under, which is not ours.
    pub licence: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// The published list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub models: Vec<Published>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ModelError::Io(path.display().to_string(), e.to_string()))?;
        serde_yaml::from_str(&text)
            .map_err(|e| ModelError::Malformed(path.display().to_string(), e.to_string()))
    }

    pub fn by_hash(&self, sha256: &str) -> Option<&Published> {
        self.models.iter().find(|m| m.sha256.eq_ignore_ascii_case(sha256))
    }

    pub fn for_seam(&self, seam: Seam) -> impl Iterator<Item = &Published> {
        self.models.iter().filter(move |m| m.seam == seam)
    }
}

/// What was actually loaded, as it will appear in a decision record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub seam: Seam,
    /// The published id where the hash was recognised, otherwise `None`.
    pub id: Option<String>,
    pub file: String,
    pub sha256: String,
    /// False when the hash matched nothing published. Not an error.
    pub verified: bool,
}

impl Provenance {
    /// How this reads in a record.
    pub fn describe(&self) -> String {
        match (&self.id, self.verified) {
            (Some(id), true) => format!("{id} ({})", short(&self.sha256)),
            _ => format!("unverified {} ({})", self.file, short(&self.sha256)),
        }
    }
}

fn short(sha256: &str) -> &str {
    &sha256[..sha256.len().min(12)]
}

/// SHA-256 of a file, streamed so a multi-gigabyte model does not have to be
/// held in memory to be identified.
pub fn hash_file(path: &Path) -> Result<String, ModelError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| ModelError::Io(path.display().to_string(), e.to_string()))?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| ModelError::Io(path.display().to_string(), e.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Identify a file, against the published list.
pub fn identify(seam: Seam, path: &Path, manifest: &Manifest) -> Result<Provenance, ModelError> {
    let sha256 = hash_file(path)?;
    let known = manifest.by_hash(&sha256);

    Ok(Provenance {
        seam,
        id: known.map(|k| k.id.clone()),
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sha256,
        verified: known.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A file per call, not per test run. These tests run in parallel and an
    /// earlier version shared one path, so the test that tampers with a file
    /// raced the test that expects it intact.
    fn temp_file(contents: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let mut path = std::env::temp_dir();
        path.push(format!("adi-models-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path.push(format!("weights-{}.bin", NEXT.fetch_add(1, Ordering::Relaxed)));

        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    /// The published vector for SHA-256 of the empty input, so a disagreement
    /// with any other implementation shows up here.
    #[test]
    fn the_hash_is_a_real_sha256() {
        let path = temp_file(b"");
        assert_eq!(
            hash_file(&path).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_recognised_file_is_named() {
        let path = temp_file(b"weights");
        let sha256 = hash_file(&path).unwrap();

        let manifest = Manifest {
            models: vec![Published {
                id: "example-1b-q4".into(),
                seam: Seam::Interpreter,
                file: "weights.bin".into(),
                sha256: sha256.clone(),
                bytes: 7,
                source: "https://example.invalid/weights.bin".into(),
                licence: "Apache-2.0".into(),
                notes: None,
            }],
        };

        let found = identify(Seam::Interpreter, &path, &manifest).unwrap();
        assert!(found.verified);
        assert_eq!(found.id.as_deref(), Some("example-1b-q4"));
        assert!(found.describe().starts_with("example-1b-q4"));
    }

    /// The property that makes a custom model possible. An unknown file loads.
    #[test]
    fn an_unrecognised_file_loads_and_is_recorded_as_unverified() {
        let path = temp_file(b"somebody elses model");
        let found = identify(Seam::Interpreter, &path, &Manifest::default()).unwrap();

        assert!(!found.verified, "not recognised");
        assert!(found.id.is_none(), "and so it has no published id");
        assert!(
            found.describe().contains("unverified"),
            "but it is still identifiable afterwards: {}",
            found.describe()
        );
        assert_eq!(found.sha256.len(), 64, "by its hash");
    }

    /// A file that has changed since it was published is not the file that was
    /// published, and the record must not claim otherwise.
    #[test]
    fn a_modified_file_stops_being_recognised() {
        let path = temp_file(b"weights");
        let manifest = Manifest {
            models: vec![Published {
                id: "example-1b-q4".into(),
                seam: Seam::Interpreter,
                file: "weights.bin".into(),
                sha256: hash_file(&path).unwrap(),
                bytes: 7,
                source: "https://example.invalid/weights.bin".into(),
                licence: "Apache-2.0".into(),
                notes: None,
            }],
        };
        assert!(identify(Seam::Interpreter, &path, &manifest).unwrap().verified);

        let changed = temp_file(b"weights, modified since");
        assert!(!identify(Seam::Interpreter, &changed, &manifest).unwrap().verified);
    }

    #[test]
    fn a_missing_file_is_an_error_rather_than_an_empty_hash() {
        let missing = std::path::Path::new("no-such-model.gguf");
        assert!(matches!(hash_file(missing), Err(ModelError::Io(_, _))));
    }
}

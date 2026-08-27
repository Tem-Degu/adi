//! Knowledge extensions and layering. ADI-P section 6.
//!
//! Layers resolve highest first. A higher layer may add entries and narrow
//! ranges. It may never widen one, and it may never touch the safety envelope.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::VariableId;
use crate::units::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// StreetAI. The domain knowledge base.
    Base = 0,
    /// Manufacturer additions, namespaced.
    Vendor = 1,
    /// Integrator or region.
    Deployment = 2,
    /// Learned from this user.
    User = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeExtension {
    pub namespace: String,
    pub extension_version: String,
    pub extends: Extends,
    #[serde(default)]
    pub styles: Vec<StyleEntry>,
    #[serde(default)]
    pub material_priors: Vec<MaterialPrior>,
    #[serde(default)]
    pub defaults: Vec<DefaultEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extends {
    pub domain_profile: String,
    pub base_min_version: String,
}

/// A named outcome style with its target ranges. In espresso a style is a drink;
/// in another domain it is whatever the profile says it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleEntry {
    pub id: String,
    #[serde(default)]
    pub inherits: Option<String>,
    #[serde(default)]
    pub targets: BTreeMap<String, Range>,
}

/// Prior expectations for a class of input material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialPrior {
    pub id: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub offsets: BTreeMap<String, f64>,
    #[serde(default)]
    pub targets: BTreeMap<String, Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultEntry {
    pub variable: VariableId,
    pub value: f64,
    pub unit: crate::units::Unit,
    #[serde(default)]
    pub rationale_note_id: Option<String>,
}

/// A value together with the layer that produced it.
///
/// ADI-P 9.3 requires every decision to be reconstructable, and layer
/// attribution is what makes "the dose came from the vendor default, the
/// temperature from the user's learned preference" possible.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Attributed<T> {
    pub value: T,
    pub layer: Layer,
}

impl<T> Attributed<T> {
    pub fn new(value: T, layer: Layer) -> Self {
        Self { value, layer }
    }
}

/// Resolves a target across layers.
///
/// Constraint monotonicity, ADI-P 6.4: the effective range is the intersection
/// of every layer. Any layer that would widen the range is a validation error,
/// not a silent override.
#[derive(Debug, Clone, Default)]
pub struct ResolvedTargets {
    entries: BTreeMap<String, Attributed<Range>>,
}

impl ResolvedTargets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one layer's contribution. Returns an error if the layer widens an
    /// existing range or changes its unit.
    pub fn apply(
        &mut self,
        key: &str,
        range: Range,
        layer: Layer,
    ) -> Result<(), MonotonicityError> {
        match self.entries.get(key) {
            None => {
                self.entries
                    .insert(key.to_string(), Attributed::new(range, layer));
                Ok(())
            }
            Some(existing) => {
                let narrowed = existing.value.intersect(&range).ok_or_else(|| {
                    MonotonicityError::Incompatible {
                        key: key.to_string(),
                        layer,
                    }
                })?;
                if narrowed.min < existing.value.min || narrowed.max > existing.value.max {
                    return Err(MonotonicityError::Widened {
                        key: key.to_string(),
                        layer,
                    });
                }
                self.entries
                    .insert(key.to_string(), Attributed::new(narrowed, layer));
                Ok(())
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Attributed<Range>> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Attributed<Range>)> {
        self.entries.iter()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum MonotonicityError {
    #[error("layer {layer:?} widens the range for '{key}'; higher layers may only narrow")]
    Widened { key: String, layer: Layer },
    #[error("layer {layer:?} declares an incompatible or empty range for '{key}'")]
    Incompatible { key: String, layer: Layer },
}

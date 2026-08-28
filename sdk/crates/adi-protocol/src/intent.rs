//! Structured intent. What a person wants, as data.
//!
//! This is the closed schema ADI-P 9.3 requires: the only thing a language
//! model is permitted to emit on the request path. It describes a goal, never
//! an action. There is no tool name in here and no way to write one.
//!
//! The important design point is that a target may be **relative**. "Half the
//! sugar of last time" is expressible without the model knowing what last time
//! was, and without it doing the arithmetic. It says `scale: 0.5, of: last`,
//! and the engine resolves that against what it remembers. A model that had to
//! emit `4 g` would have to be trusted to remember correctly and divide
//! correctly, and would be wrong silently when it did not.
//!
//! Whatever this schema cannot express, the product cannot do. That is the
//! reason to look at it carefully.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::VariableId;
use crate::units::Quantity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// Run a cycle and produce something.
    Produce,
    /// Change how the next one comes out, without necessarily running now.
    Adjust,
}

/// One request, as structured data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub kind: IntentKind,
    /// The outcome style asked for, if the person named one.
    #[serde(default)]
    pub style: Option<String>,
    /// The input material, if named.
    #[serde(default)]
    pub material: Option<String>,
    /// What the person asked to be different, by variable.
    #[serde(default)]
    pub targets: BTreeMap<VariableId, TargetSpec>,
}

/// What a variable should be.
///
/// Untagged, so the two forms are told apart by their fields rather than by a
/// label. An absolute target is `{ value, unit }` and a relative one names what
/// it is relative to with `of`. That keeps the schema readable for the people
/// who write it by hand, which is manufacturers and integrators.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetSpec {
    /// A stated quantity. `{ value: 120, unit: ml }`.
    Absolute(Quantity),
    /// Expressed against something remembered. `{ of: last, scale: 0.5 }`.
    Relative(RelativeSpec),
}

/// A target given in terms of an earlier cycle.
///
/// `scale` and `offset` are separate optional fields rather than an enum,
/// because that makes the YAML a manufacturer or an integrator reads plain, and
/// because exactly-one-of is easy to check and to report on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelativeSpec {
    pub of: Reference,
    /// Multiplier. Half is 0.5, double is 2.0, none at all is 0.0.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Additive change, in the variable's own unit. "Two grams more."
    #[serde(default)]
    pub offset: Option<Quantity>,
}

/// Which earlier cycle a relative target is measured against.
///
/// Written as a plain string: `of: last`, or `of: "<cycle id>"`. A tagged form
/// would be unambiguous but cannot appear inside an untagged enum, and this
/// reads better for the people writing it besides.
///
/// **`last` is reserved.** A cycle id equal to "last" is read as the most
/// recent cycle, so ids must not use that word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Reference {
    /// The most recent cycle. "Last time."
    Last,
    /// A specific one. "The one on Tuesday", once the language layer has
    /// resolved which that was.
    Cycle(String),
}

pub const REFERENCE_LAST: &str = "last";

impl From<String> for Reference {
    fn from(s: String) -> Self {
        if s == REFERENCE_LAST {
            Reference::Last
        } else {
            Reference::Cycle(s)
        }
    }
}

impl From<Reference> for String {
    fn from(r: Reference) -> Self {
        match r {
            Reference::Last => REFERENCE_LAST.to_string(),
            Reference::Cycle(id) => id,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum IntentError {
    #[error("an intent to adjust names no targets, so there is nothing to do")]
    NoTargets,
    #[error("target '{0}' gives both a scale and an offset; it must give one or neither")]
    ScaleAndOffset(String),
    #[error("target '{variable}' has a {kind} that is not a usable number")]
    NotFinite { variable: String, kind: &'static str },
    #[error("target '{0}' has a negative scale, which has no meaning for a quantity")]
    NegativeScale(String),
}

impl Intent {
    /// Structural validation, before anything acts on it.
    ///
    /// 9.3 requires a model's output be validated against this schema before
    /// use. Parsing enforces the shape; this enforces the rest.
    pub fn validate(&self) -> Result<(), IntentError> {
        if self.kind == IntentKind::Adjust && self.targets.is_empty() {
            return Err(IntentError::NoTargets);
        }

        for (variable, spec) in &self.targets {
            let TargetSpec::Relative(rel) = spec else {
                if let TargetSpec::Absolute(q) = spec {
                    if !q.value.is_finite() {
                        return Err(IntentError::NotFinite {
                            variable: variable.0.clone(),
                            kind: "value",
                        });
                    }
                }
                continue;
            };

            if rel.scale.is_some() && rel.offset.is_some() {
                return Err(IntentError::ScaleAndOffset(variable.0.clone()));
            }
            if let Some(scale) = rel.scale {
                if !scale.is_finite() {
                    return Err(IntentError::NotFinite {
                        variable: variable.0.clone(),
                        kind: "scale",
                    });
                }
                // Zero is meaningful: "no sugar at all". Negative is not.
                if scale < 0.0 {
                    return Err(IntentError::NegativeScale(variable.0.clone()));
                }
            }
            if let Some(offset) = rel.offset {
                if !offset.value.is_finite() {
                    return Err(IntentError::NotFinite {
                        variable: variable.0.clone(),
                        kind: "offset",
                    });
                }
            }
        }
        Ok(())
    }
}

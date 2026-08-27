//! The skill. ADI-P section 5.
//!
//! How a machine responds. Descriptive only: a skill states what happens, never
//! what the engine should do. That separation is what keeps the reasoning
//! portable across machines and the audit trail intact.

use serde::{Deserialize, Serialize};

use crate::ids::{SemanticId, VariableId};
use crate::units::Unit;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub skill_version: String,
    pub skill_id: String,
    pub applies_to: AppliesTo,
    #[serde(default)]
    pub responses: Vec<Response>,
    #[serde(default)]
    pub dynamics: Vec<Dynamic>,
    #[serde(default)]
    pub interactions: Vec<Interaction>,
    #[serde(default)]
    pub operating: Option<Operating>,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliesTo {
    pub vendor: String,
    pub model: String,
    #[serde(default)]
    pub firmware_range: Option<FirmwareRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareRange {
    pub min: String,
    pub max: String,
}

/// How far a domain variable moves per unit of a tool parameter.
///
/// `per_unit` is signed rather than described in words, so direction cannot be
/// misread. Negative means increasing the parameter decreases the variable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub tool: SemanticId,
    pub parameter: String,
    pub variable: VariableId,
    pub per_unit: f64,
    pub unit: Unit,
    /// Where the stated linearity holds. Outside it the engine falls back to
    /// conservative defaults rather than extrapolating.
    #[serde(default)]
    pub valid_range: Option<ValidRange>,
    /// 0 to 1. Sizes correction steps and decides how far to trust the declared
    /// figure against the engine's own observations.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    0.5
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ValidRange {
    pub min: f64,
    pub max: f64,
}

impl ValidRange {
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// When a change does not take effect immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dynamic {
    #[serde(default)]
    pub tool: Option<SemanticId>,
    #[serde(default)]
    pub scope: Option<String>,
    /// Material held over from the previous setting, so the next cycle is a
    /// blend. An engine unaware of this misreads the result and corrects in the
    /// wrong direction.
    #[serde(default)]
    pub retention: Option<Retention>,
    #[serde(default)]
    pub settling: Option<Settling>,
    /// Mechanisms that must be approached from one direction.
    #[serde(default)]
    pub hysteresis: Option<Hysteresis>,
    #[serde(default)]
    pub warmup: Option<Settling>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retention {
    pub mass: Measured,
    #[serde(default)]
    pub remedy_tool: Option<SemanticId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settling {
    pub time: Measured,
    #[serde(default)]
    pub tolerance: Option<Measured>,
    #[serde(default)]
    pub note_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hysteresis {
    pub steps: f64,
    pub direction: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Measured {
    pub value: f64,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub changing: VariableId,
    /// Stored comparison points this change makes invalid.
    #[serde(default)]
    pub invalidates: Vec<String>,
    /// Other variables this change moves. Counts toward the
    /// one-variable-at-a-time rule in ADI-P 9.4.
    #[serde(default)]
    pub also_affects: Vec<VariableId>,
}

/// Where the machine performs well, as opposed to where it is safe. Soft
/// boundaries: the engine may cross one when nothing else solves the problem,
/// and should say so. It may never cross the safety envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operating {
    #[serde(default)]
    pub reliable_ranges: Vec<ReliableRange>,
    #[serde(default)]
    pub recovery: Option<Recovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliableRange {
    pub variable: VariableId,
    pub min: f64,
    pub max: f64,
    pub unit: Unit,
    #[serde(default)]
    pub note_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recovery {
    #[serde(default)]
    pub between_cycles: Option<Measured>,
}

/// Advisory and non-normative. May be surfaced to the operator as explanation.
/// MUST NOT be an input to any actuation value. See ADI-P 5.7.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
}

impl Skill {
    /// Every declared response for a tool.
    pub fn responses_for<'a>(
        &'a self,
        tool: &'a SemanticId,
    ) -> impl Iterator<Item = &'a Response> + 'a {
        self.responses.iter().filter(move |r| &r.tool == tool)
    }

    /// The response describing how `tool` moves `variable`, if declared.
    pub fn response(&self, tool: &SemanticId, variable: &VariableId) -> Option<&Response> {
        self.responses
            .iter()
            .find(|r| &r.tool == tool && &r.variable == variable)
    }

    /// Tools with a declared effect on `variable`. This is what makes a
    /// vendor tool selectable: without a response, the engine has no basis to
    /// choose it. See ADI-P 8.2.
    pub fn tools_affecting(&self, variable: &VariableId) -> Vec<&SemanticId> {
        self.responses
            .iter()
            .filter(|r| &r.variable == variable)
            .map(|r| &r.tool)
            .collect()
    }

    pub fn retention_for(&self, tool: &SemanticId) -> Option<&Retention> {
        self.dynamics
            .iter()
            .find(|d| d.tool.as_ref() == Some(tool))
            .and_then(|d| d.retention.as_ref())
    }
}

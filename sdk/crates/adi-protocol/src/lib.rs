//! ADI-P wire types.
//!
//! Parsing and validation of the three documents a manufacturer supplies:
//! the capability declaration (what the machine can do), the skill (how it
//! behaves) and the knowledge extension (what it adds to the domain knowledge).
//!
//! This crate is domain-neutral. It knows nothing about any particular
//! appliance and must never learn.

pub mod declaration;
pub mod ids;
pub mod invocation;
pub mod knowledge;
pub mod skill;
pub mod units;

pub use declaration::{Declaration, ConformanceLevel, Tool, Signal, Parameter};
pub use ids::{SemanticId, VariableId};
pub use invocation::{Invocation, Outcome, RejectReason};
pub use knowledge::{Attributed, KnowledgeExtension, Layer};
pub use skill::{Response, Skill};
pub use units::{Quantity, Range, Unit};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The safety envelope as declared. Enforcement lives in `adi-safety`; this is
/// only the parsed form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeSpec {
    pub enforced_by: EnforcedBy,
    /// MUST be true to claim conformance at any level. An appliance whose
    /// safety depends on the engine behaving correctly does not conform.
    pub independent_of_engine: bool,
    #[serde(default)]
    pub limits: Vec<HardLimit>,
    #[serde(default)]
    pub interlocks: Vec<Interlock>,
    /// Stateful constraints. Individually valid actions can compose into an
    /// unsafe sequence, and per-call limits cannot express that.
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub rate_limits: Vec<RateLimit>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    pub failsafe: Failsafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcedBy {
    Firmware,
    Hardware,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardLimit {
    pub parameter: String,
    pub unit: Unit,
    #[serde(default)]
    pub hard_min: Option<f64>,
    #[serde(default)]
    pub hard_max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interlock {
    pub id: String,
    /// Tool ids this interlock blocks. `["*"]` blocks everything.
    pub blocks: Vec<String>,
}

/// A condition that must hold at all times, not only at the moment of a call.
///
/// This is what stops three individually valid dispense calls from overflowing
/// a cup. Cup capacity is not a property of any single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invariant {
    pub id: String,
    pub quantity: String,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
    pub unit: Unit,
    /// Tools that contribute to this quantity, and by which parameter.
    #[serde(default)]
    pub accumulates: BTreeMap<String, String>,
    /// What resets the accumulator, for example ejecting the cup.
    #[serde(default)]
    pub reset_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    pub id: String,
    /// Tool ids this limit applies to.
    pub tools: Vec<String>,
    #[serde(default)]
    pub max_invocations: Option<u32>,
    #[serde(default)]
    pub window_s: Option<f64>,
    #[serde(default)]
    pub min_interval_s: Option<f64>,
    #[serde(default)]
    pub max_duty_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: String,
    pub unit: Unit,
    #[serde(default)]
    pub capacity: Option<f64>,
    #[serde(default)]
    pub consumed_by: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failsafe {
    pub on_control_timeout: String,
    pub on_signal_loss: String,
    pub on_fault: String,
    pub safe_state: String,
    pub max_time_to_safe_ms: u64,
}

/// Errors from parsing or validating a protocol document.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported adip_version '{0}'")]
    UnsupportedVersion(String),
    #[error("malformed identifier '{0}'")]
    MalformedId(String),
    #[error("identifier '{id}' is outside vendor namespace '{namespace}'")]
    NamespaceViolation { id: String, namespace: String },
    #[error("vendor extension may not redefine base identifier '{0}'")]
    Redefinition(String),
    #[error("envelope declares independent_of_engine = false")]
    EnvelopeNotIndependent,
    #[error("conformance level {level:?} requires tool '{tool}'")]
    MissingRequiredTool { level: String, tool: String },
    #[error("procedure '{procedure}' fixes outcome variable '{variable}'")]
    OutcomeHardcoded { procedure: String, variable: String },
    #[error(transparent)]
    Monotonicity(#[from] knowledge::MonotonicityError),
    #[error("{0}")]
    Other(String),
}

/// The protocol version this build implements.
pub const ADIP_VERSION: &str = "0.2";

impl Declaration {
    /// Structural validation. Section 9.1 requires this before any invocation.
    ///
    /// Deliberately does not check envelope enforcement, which cannot be
    /// verified from a document and belongs in the conformance test suite.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let major = self.adip_version.split('.').next().unwrap_or("");
        if major != "0" {
            return Err(ProtocolError::UnsupportedVersion(self.adip_version.clone()));
        }
        if !self.envelope.independent_of_engine {
            return Err(ProtocolError::EnvelopeNotIndependent);
        }
        for tool in &self.tools {
            if !tool.id.is_well_formed() {
                return Err(ProtocolError::MalformedId(tool.id.0.clone()));
            }
        }
        for signal in &self.signals {
            if !signal.id.is_well_formed() {
                return Err(ProtocolError::MalformedId(signal.id.0.clone()));
            }
        }
        Ok(())
    }

    pub fn tool(&self, id: &SemanticId) -> Option<&Tool> {
        self.tools.iter().find(|t| &t.id == id)
    }

    pub fn signal(&self, id: &SemanticId) -> Option<&Signal> {
        self.signals.iter().find(|s| &s.id == id)
    }

    /// Tools the engine is permitted to invoke.
    pub fn engine_tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools
            .iter()
            .filter(|t| t.authority.contains(&declaration::Authority::Engine))
    }

    /// True when the declaration carries the stateful constraints that D2
    /// requires. See `adi-decision`.
    pub fn has_stateful_envelope(&self) -> bool {
        !self.envelope.invariants.is_empty() || !self.envelope.rate_limits.is_empty()
    }
}

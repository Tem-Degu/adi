//! The capability declaration. ADI-P section 4.
//!
//! What a machine can do. Nothing about how it behaves: that is the skill.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::{SemanticId, VariableId};
use crate::units::{Monotonic, Range, Unit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceLevel {
    /// Advisory. Signals only, no actuation.
    Advisory,
    /// Parameterised control. Operator still prepares.
    Parameterised,
    /// Autonomous. No human contact required.
    Autonomous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Declaration {
    pub adip_version: String,
    pub domain: String,
    pub domain_profile: String,
    pub conformance_level: ConformanceLevel,
    pub device: Device,
    pub session: Session,
    #[serde(default)]
    pub references: References,
    #[serde(default)]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub events: Vec<Event>,
    pub envelope: crate::EnvelopeSpec,
    #[serde(default)]
    pub procedures: Vec<Procedure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub vendor: String,
    pub model: String,
    #[serde(default)]
    pub hardware_revision: Option<String>,
    pub firmware_version: String,
    #[serde(default)]
    pub serial: Option<String>,
    /// Changes whenever any part of the declaration changes.
    pub declaration_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub max_concurrent: u32,
    pub heartbeat_interval_ms: u64,
    pub control_timeout_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct References {
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub knowledge_extension: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: SemanticId,
    #[serde(default)]
    pub name: Option<String>,
    pub unit: Unit,
    #[serde(default)]
    pub range: Option<Range>,
    pub sample_rate_hz: f64,
    /// Mandatory. Without it the engine must behave conservatively, which
    /// makes the machine look worse than it is. See ADI-P 4.3.
    pub accuracy: Accuracy,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub derived_from: Option<SemanticId>,
    #[serde(default)]
    pub filtered: bool,
    #[serde(default)]
    pub available: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Accuracy {
    pub abs: f64,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub id: SemanticId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// Domain variables this tool changes. Used by the one-variable-at-a-time
    /// rule in ADI-P 9.5.
    #[serde(default)]
    pub effects: Vec<VariableId>,
    #[serde(default)]
    pub duration_ms_typ: Option<u64>,
    #[serde(default)]
    pub duration_ms_max: Option<u64>,
    #[serde(default = "default_true")]
    pub reversible: bool,
    #[serde(default)]
    pub abortable: bool,
    #[serde(default)]
    pub implicit: bool,
    #[serde(default)]
    pub hazard: Vec<String>,
    #[serde(default = "default_authority")]
    pub authority: Vec<Authority>,
}

fn default_true() -> bool {
    true
}

fn default_authority() -> Vec<Authority> {
    vec![Authority::Engine, Authority::Operator]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    Engine,
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Parameter {
    Number {
        unit: Unit,
        min: f64,
        max: f64,
        #[serde(default)]
        resolution: Option<f64>,
        #[serde(default)]
        default: Option<f64>,
        #[serde(default)]
        optional: bool,
    },
    Integer {
        unit: Unit,
        min: i64,
        max: i64,
        #[serde(default)]
        resolution: Option<i64>,
        #[serde(default)]
        monotonic: Option<Monotonic>,
        #[serde(default)]
        optional: bool,
    },
    Enum {
        values: Vec<String>,
        #[serde(default)]
        default: Option<String>,
        #[serde(default)]
        optional: bool,
    },
    Object {
        schema: String,
        #[serde(default)]
        optional: bool,
    },
}

impl Parameter {
    pub fn range(&self) -> Option<Range> {
        match self {
            Parameter::Number {
                unit,
                min,
                max,
                resolution,
                ..
            } => Some(Range {
                min: *min,
                max: *max,
                resolution: *resolution,
                unit: *unit,
            }),
            Parameter::Integer {
                unit,
                min,
                max,
                resolution,
                ..
            } => Some(Range {
                min: *min as f64,
                max: *max as f64,
                resolution: resolution.map(|r| r as f64).or(Some(1.0)),
                unit: *unit,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: SemanticId,
    #[serde(default)]
    pub payload: BTreeMap<String, Parameter>,
}

/// Escape hatch only. ADI-P 8.3. Most sequencing should emerge from reasoning
/// over tools, skill and knowledge rather than from a declared sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    pub id: SemanticId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub produces: Option<SemanticId>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub effects: Vec<VariableId>,
    #[serde(default)]
    pub abortable_until: Option<String>,
    pub max_duration_s: f64,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub tool: SemanticId,
    #[serde(default)]
    pub arguments: BTreeMap<String, Binding>,
    #[serde(default)]
    pub await_on: Option<Await>,
    #[serde(default)]
    pub on_error: OnError,
}

/// Where an argument's value comes from. The binding rule in ADI-P 8.3.2 says
/// an argument naming a domain variable MUST be `Engine`, so a manufacturer
/// cannot hardcode an outcome and bypass the reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "binding", rename_all = "snake_case")]
pub enum Binding {
    Fixed { value: serde_json::Value },
    Parameter { parameter: String },
    Operator,
    Engine { domain_variable: VariableId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Await {
    pub signal: SemanticId,
    pub condition: AwaitCondition,
    pub timeout_s: f64,
    pub on_timeout: OnError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitCondition {
    Stable,
    Rising,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    #[default]
    Abort,
    Continue,
}

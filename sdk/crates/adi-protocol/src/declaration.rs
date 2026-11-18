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
    /// The outcome this signal measures, where it measures one.
    ///
    /// A signal says what the machine senses. A variable says what the engine
    /// reasons about. Nothing connected the two, so a reading arrived as
    /// "sensor 0 reads 15.0 pct" and the engine had no way to know that sensor
    /// was the thing a request had just named. It planned from the last
    /// observed cycle instead, and on a machine that has run none it planned
    /// from nothing: a request for a change computed a change of zero, which
    /// is accepted by the appliance and moves nothing.
    ///
    /// Optional, because a signal need not measure an outcome. Air
    /// temperature on its way to a setpoint is worth publishing and is not
    /// something anyone asks for. One that does name an outcome must be in
    /// that outcome's unit, which `adi validate` checks where the knowledge is
    /// resolved and the session checks again before using a value.
    #[serde(default)]
    pub measures: Option<SemanticId>,
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
    /// What this tool is and when to prefer it, for a model choosing between
    /// candidates. Advisory, and read by no engine logic.
    ///
    /// A description MAY influence which candidate a model picks. It MUST NOT
    /// affect which candidates exist, or what value is sent. Candidates are
    /// derived from `effects` and the skill's declared response; the value is
    /// clamped to the declared range and snapped to the declared resolution
    /// whatever any text says. So a description that is wrong, stale or
    /// promotional can at worst cause a suboptimal choice among options that
    /// are already safe, and cannot reach the actuator.
    ///
    /// The physics does not belong here. A description stating "moves shot time
    /// by 1.4 seconds per step" puts that number in two places, and the skill's
    /// `per_unit` is the one the engine uses. Describe what the mechanism is,
    /// what it settles like, and when another tool would serve better.
    #[serde(default)]
    pub description: Option<String>,
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

/// One action an operator may ask for by name. ADI-P 8.2.
///
/// Not everything an appliance does is a quantity. `brew.start` moves a number
/// and a skill response can describe it, so the planner reaches it. Stopping,
/// declaring a fresh cup and running a rinse move nothing: they have no
/// parameters, a skill response requires one, so no response can exist and the
/// planner can never select them. 8.2 nonetheless says a declared tool is
/// invocable "when the operator names it", and this is the list of the ones
/// that applies to.
#[derive(Debug, Clone)]
pub struct OperatorAction<'a> {
    /// Position in this list. What a model is permitted to return, in place of
    /// a tool name it is not: ADI-P 9.3 requirement 3.
    pub index: usize,
    pub tool: &'a SemanticId,
    /// For the person and for the model. The tool's `name` where it declares
    /// one, otherwise the last segment of its identifier.
    pub name: &'a str,
    pub description: Option<&'a str>,
    /// Non-empty where the declaration flags a hazard, which 4.2 says informs
    /// confirmation policy. A caller SHOULD confirm before performing one.
    pub hazard: &'a [String],
    /// The outcomes this action changes, which is how a caller tells a drink
    /// from a procedure.
    ///
    /// Both can be hazardous and the hazard alone cannot separate them: every
    /// drink this machine makes involves hot liquid. What separates them is
    /// whether anything arrives. An action that delivers something is the thing
    /// that was asked for; one that delivers nothing and carries a hazard is a
    /// routine, and a routine is worth a question.
    pub effects: &'a [VariableId],
}

impl Declaration {
    /// The actions an operator may name, in declaration order.
    ///
    /// Two conditions, and the second is the safety of the whole mechanism.
    ///
    /// The tool must be declared for operator authority. And it must be
    /// invocable **with no arguments at all**, meaning every parameter it
    /// declares is optional.
    ///
    /// That second condition is what makes this path incapable of carrying a
    /// value. Every argument sent is one the manufacturer declared as a
    /// default, so a model choosing from this list cannot contribute a number
    /// even if it tried: there is nowhere in the arm to put one, and nothing
    /// here that would read it. A tool needing a magnitude is excluded, which
    /// is correct, because deciding a magnitude is the deterministic engine's
    /// job and 9.3 keeps it there.
    ///
    /// Optional, specifically, rather than merely defaulted. A parameter with a
    /// default that is not marked optional is still required: `adip_validate.c`
    /// tests only `ADIP_PARAM_F_OPTIONAL`, and an invocation missing such an
    /// argument is refused - reported, confusingly, as a precondition failure.
    /// Reading `default` here instead would build a list of actions the
    /// appliance rejects.
    pub fn operator_actions(&self) -> Vec<OperatorAction<'_>> {
        self.tools
            .iter()
            .filter(|tool| {
                tool.authority.contains(&Authority::Operator)
                    && tool.parameters.values().all(Parameter::is_optional)
            })
            .enumerate()
            .map(|(index, tool)| OperatorAction {
                index,
                tool: &tool.id,
                name: tool.name.as_deref().unwrap_or_else(|| {
                    tool.id.as_str().rsplit('.').next().unwrap_or(tool.id.as_str())
                }),
                description: tool.description.as_deref(),
                hazard: &tool.hazard,
                effects: &tool.effects,
            })
            .collect()
    }

    /// Vendor tools an operator may name that say nothing about what they do.
    ///
    /// The same rule as `Skill::undescribed_vendor_tools`, for the same stated
    /// reason, applied to the other way a model reaches a tool.
    ///
    /// That rule is scoped to tools a skill makes selectable, because a
    /// response is what lets the planner choose one. A namable action can
    /// never have a response - having no parameter is the whole reason it is
    /// namable - so the existing check structurally never covers it, and a
    /// vendor action with no description was legal while being handed to a
    /// model to pick from.
    ///
    /// A registry identifier is exempt because it means the same on every
    /// machine and the profile supplies its text (4.2). A vendor identifier
    /// means nothing to anyone: `x.acme.purge_3` as a bare word, in a numbered
    /// list beside "stop", is a coin toss whose outcome is a physical act.
    pub fn undescribed_named_actions(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .operator_actions()
            .iter()
            .filter(|action| action.tool.is_vendor() && action.description.is_none())
            .map(|action| action.tool.0.clone())
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

impl Parameter {
    /// Whether an invocation may leave this parameter out.
    pub fn is_optional(&self) -> bool {
        match self {
            Parameter::Number { optional, .. }
            | Parameter::Integer { optional, .. }
            | Parameter::Enum { optional, .. }
            | Parameter::Object { optional, .. } => *optional,
        }
    }

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
                // No aim. This range is what the mechanism can physically
                // do; what it is normally set to is a property of the recipe
                // being made, not of the hardware.
                aim: None,
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
                aim: None,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: SemanticId,
    #[serde(default)]
    pub payload: BTreeMap<String, EventField>,
}

/// A field in an event payload.
///
/// Deliberately not `Parameter`. ADI-P 4.2 lists the types a *tool* parameter
/// may take, and `string` and `boolean` are not among them, because every
/// numeric parameter must carry a unit, a range and a resolution and neither
/// of those can. An event reports what happened rather than requesting an
/// action, so it carries codes and flags that no tool may accept.
///
/// The fault event of 4.5 is the case that forced the distinction: it reports
/// a `code` and whether the condition is `recoverable`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventField {
    Number {
        unit: Unit,
        #[serde(default)]
        range: Option<Range>,
    },
    Integer {
        unit: Unit,
        #[serde(default)]
        range: Option<Range>,
    },
    Enum {
        values: Vec<String>,
    },
    String,
    Boolean,
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

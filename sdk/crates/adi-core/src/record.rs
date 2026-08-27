//! Decision records. ADI-P 9.3 requirement 5.
//!
//! Every invocation must be reconstructable: the inputs, the layer that
//! produced each value used, and the rule that produced the decision. This is
//! what answers the question a regulator or insurer eventually asks, which is
//! why the machine did that.
//!
//! Layer attribution is the part people skip and then cannot retrofit. Build it
//! in from the first commit.

use std::collections::BTreeMap;

use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::knowledge::Layer;
use adi_protocol::units::Quantity;

/// Where one value came from.
#[derive(Debug, Clone)]
pub struct ValueProvenance {
    pub variable: VariableId,
    pub value: Quantity,
    pub layer: Layer,
    /// Skill response, registry default, or profile rule.
    pub source: ValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueSource {
    /// Computed from a declared skill response.
    SkillResponse { tool: SemanticId, parameter: String },
    /// Profile default, used when no skill response covered the variable.
    ProfileDefault,
    /// Read from a resolved knowledge layer.
    Knowledge { key: String },
    /// Measured from a signal this cycle.
    Measured { signal: SemanticId },
    /// Supplied by the operator.
    Operator,
}

/// A complete account of one decision.
#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub cycle_id: String,
    pub decision_id: String,
    /// Which autonomy level produced this. See `adi-decision`.
    pub autonomy_level: &'static str,
    /// The diagnosed cause.
    pub cause: String,
    /// The rule identifier the profile used.
    pub rule_id: String,
    /// Tool selected, and by whom.
    pub selected_tool: Option<SemanticId>,
    pub selected_by: SelectedBy,
    /// Every value that fed the decision, with its layer.
    pub inputs: Vec<ValueProvenance>,
    /// Arguments actually sent.
    pub arguments: BTreeMap<String, Quantity>,
    /// Values before clamping, where clamping changed them.
    pub clamped_from: BTreeMap<String, f64>,
    /// Explanation for the language layer. Never an actuation input.
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedBy {
    /// Deterministic engine chose the tool.
    Engine,
    /// Model chose the tool from declared options.
    Model,
}

impl DecisionRecord {
    /// True when this decision can be replayed exactly. A model-selected tool
    /// makes the selection non-reproducible even though the values remain
    /// deterministic, which is the honest cost of D1.
    pub fn is_reproducible(&self) -> bool {
        self.selected_by == SelectedBy::Engine
    }
}

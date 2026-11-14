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
    /// Which recipe this was serving, where it was serving one.
    ///
    /// WHY THE TOOL IS NOT ENOUGH. `selected_tool` says which mechanism moved.
    /// On a machine where one mechanism serves many recipes it does not say
    /// what was being made: a lungo, an americano, a doppio and a plain coffee
    /// all come out of `adi.espresso.brew.start` with a different number of
    /// millilitres, and the number alone does not name the drink.
    ///
    /// So looking back at its own records, an engine could say that 118
    /// millilitres had gone through the brewer and could not say it had made a
    /// lungo. Asked for "another one" it had nothing to repeat, and guessed -
    /// reliably picking one of the machine's NAMED products, because those are
    /// the only decisions whose records identify themselves.
    ///
    /// The style was known when the decision was taken and thrown away a few
    /// lines later. ADI-P 9.3 asks a record to account for the decision; what
    /// it was aiming to produce is part of that account.
    ///
    /// `None` where no recipe was involved: a named action is already
    /// identified by its own tool, and an adjustment to one variable is not
    /// serving a recipe at all.
    pub style: Option<String>,

    /// Explanation for the language layer. Never an actuation input.
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedBy {
    /// Deterministic engine chose the tool.
    Engine,
    /// Model chose the tool from declared options.
    Model,
    /// A person named one of the machine's declared actions, and the engine
    /// carried it out.
    ///
    /// Distinct from `Model` even though a model usually read the words. The
    /// model only said *which* of the actions the engine had offered was meant;
    /// it chose no tool the declaration did not already expose to an operator,
    /// and it supplied no value, because this path carries none. ADI-P 8.2
    /// permits an operator to name a declared tool, and the record has to say
    /// that is what happened: "the engine worked it out" and "someone asked for
    /// it" are different answers to why the machine did that, and §9.3
    /// requirement 5 exists so the difference can be told.
    Operator,
}

impl DecisionRecord {
    /// True when this decision can be replayed exactly. A model-selected tool
    /// makes the selection non-reproducible even though the values remain
    /// deterministic, which is the honest cost of D1.
    pub fn is_reproducible(&self) -> bool {
        self.selected_by == SelectedBy::Engine
    }
}

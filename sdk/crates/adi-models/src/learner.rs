//! Working out what a machine has learned about the person using it.
//!
//! WHAT IS LEARNED, AND WHAT IS NOT.
//!
//! A narrower range on a declared variable. Not a weight, not a model of
//! anyone - a line that says "this person takes their coffee between 1.0 and
//! 2.2 on ratio", which is a fact you can read, edit or delete.
//!
//! The distinction that falls out of ADI-P 6.4 rather than from a rule anybody
//! wrote: `constraints` are keyed by VARIABLE, so what can be learned is a
//! preference and never the thing that defines a drink. `yield_mass` is 30-42 g
//! for an espresso and 90-130 for a mug, so there is no machine-wide range to
//! narrow and it is not learnable this way. `ratio` is how strong you like it
//! whatever the size, and `brew_temperature` is how hot. Those are preferences,
//! and preferences are what a machine should be learning.
//!
//! WHY A MODEL IS ALLOWED TO JUDGE THIS.
//!
//! It proposes; it does not decide. Whatever comes back can only ever narrow
//! what was already declared - `ResolvedTargets::apply` refuses a layer that
//! widens a range, and that code was written before anyone thought about
//! learning. So a loose judgement cannot escape the manufacturer's envelope,
//! lands in a file a person can read, and is deleted by a sentence.
//!
//! Which is why there is no hard-coded "four corrections in the same
//! direction" rule here. A rule would be arbitrary, and the structure is doing
//! the work a rule would be guarding.
//!
//! It is served by the explain seam's program rather than a seam of its own.
//! Both jobs look at records after the fact and reason about them, and the
//! message shape tells them apart - the same arrangement the interpreter and
//! the chooser already share.

use std::sync::Arc;

use adi_core::record::DecisionRecord;
use serde::{Deserialize, Serialize};

use crate::backend::ProcessBackend;
use crate::ModelError;

/// One narrowed range the machine believes it has learned.
#[derive(Debug, Clone, Deserialize)]
pub struct LearnedConstraint {
    pub variable: String,
    pub min: f64,
    pub max: f64,
    pub unit: String,
}

/// What a model made of the record, and why.
#[derive(Debug, Clone, Deserialize)]
pub struct Learned {
    #[serde(default)]
    pub constraints: Vec<LearnedConstraint>,
    /// In words, for the file and for the person. A learned preference that
    /// cannot say why it exists should not have been written.
    #[serde(default)]
    pub because: String,
}

impl Learned {
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }
}

/// One decision, flattened for the wire.
///
/// Deliberately the corrections and their direction, not the whole record. A
/// preference shows itself in what was asked for repeatedly, and the inputs
/// and provenance that matter for explaining one decision are noise here.
#[derive(Debug, Serialize)]
struct Moment {
    cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    /// What was asked for, in words, as the record wrote it.
    #[serde(skip_serializing_if = "String::is_empty")]
    asked: String,
    arguments: Vec<(String, f64, String)>,
}

#[derive(Debug, Serialize)]
struct Bounds {
    variable: String,
    min: f64,
    max: f64,
    unit: String,
}

#[derive(Debug, Serialize)]
struct LearnRequest<'a> {
    learn: Ask<'a>,
}

#[derive(Debug, Serialize)]
struct Ask<'a> {
    /// What the machine has done recently, oldest first, so a direction of
    /// travel reads as one.
    moments: &'a [Moment],
    /// The ranges already in force, which a proposal may narrow and may not
    /// widen. Sent so the model is working inside them rather than being
    /// refused afterwards for not knowing them.
    current: &'a [Bounds],
}

/// Proposes what has been learned, by asking a program.
pub struct ProcessLearner {
    backend: Arc<ProcessBackend>,
}

impl ProcessLearner {
    pub fn over(backend: Arc<ProcessBackend>) -> Self {
        Self { backend }
    }

    /// Look at what has happened and say whether a preference has emerged.
    ///
    /// `current` is the ranges in force, as `(variable, min, max, unit)`.
    /// Returns an empty `Learned` where nothing has, which is the ordinary
    /// answer and not a failure: a machine that has seen four drinks from four
    /// strangers has learned nothing, and should say so.
    pub fn consider(
        &self,
        history: &[DecisionRecord],
        current: &[(String, f64, f64, String)],
    ) -> Result<Learned, ModelError> {
        // Oldest first. A model reading "stronger, stronger, stronger" needs
        // them in the order they happened; newest first reads as the opposite
        // direction of travel.
        let moments: Vec<Moment> = history
            .iter()
            .map(|record| Moment {
                cause: record.cause.clone(),
                tool: record.selected_tool.as_ref().map(|t| t.as_str().to_string()),
                asked: record.explanation.clone(),
                arguments: record
                    .arguments
                    .iter()
                    .map(|(name, quantity)| {
                        (
                            name.clone(),
                            quantity.value,
                            format!("{:?}", quantity.unit).to_lowercase(),
                        )
                    })
                    .collect(),
            })
            .collect();

        let bounds: Vec<Bounds> = current
            .iter()
            .map(|(variable, min, max, unit)| Bounds {
                variable: variable.clone(),
                min: *min,
                max: *max,
                unit: unit.clone(),
            })
            .collect();

        let request = LearnRequest {
            learn: Ask {
                moments: &moments,
                current: &bounds,
            },
        };

        let line = serde_json::to_string(&request)
            .map_err(|e| ModelError::Backend(format!("could not encode the history: {e}")))?;
        let raw = self.backend.exchange(&line).map_err(ModelError::Backend)?;

        crate::parse::json(&raw)
    }
}

/// Does a proposal narrow what is already in force?
///
/// Checked here rather than left to the layering, even though the layering
/// would refuse it. Two reasons: a proposal that could not be applied should
/// not be written to a file, and the reason it was rejected is more useful at
/// the point of proposing than at the point of loading.
///
/// Equal bounds count as narrowing - a proposal identical to what is in force
/// is pointless rather than wrong, and is filtered as empty elsewhere.
pub fn narrows(proposal: &LearnedConstraint, min: f64, max: f64, unit: &str) -> bool {
    proposal.unit.eq_ignore_ascii_case(unit)
        && proposal.min >= min
        && proposal.max <= max
        && proposal.min < proposal.max
}

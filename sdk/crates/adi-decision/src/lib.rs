//! Autonomy levels.
//!
//! Who chooses the tool, and who chooses the number.
//!
//! | Level | Tool | Value |
//! |-------|------|-------|
//! | D0    | Engine | Engine |
//! | D1    | Model, from declared tools | Engine |
//! | D2    | Model | Model |
//!
//! The level is configuration, not a separate code path. Everything below the
//! selector is identical at all three, which is what makes moving between them
//! a switch rather than a rewrite.
//!
//! D2 refuses to initialise against a declaration with no stateful invariants
//! or rate limits. Individually valid actions compose into invalid sequences,
//! and a model choosing its own magnitudes is exactly the case where that
//! matters. The refusal is the point of the level system, not an inconvenience.

use adi_core::domain::Diagnosis;
use adi_core::plan::{Candidate, Planner};
use adi_core::record::SelectedBy;
use adi_protocol::units::Quantity;
use adi_protocol::{Declaration, SemanticId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AutonomyLevel {
    /// Engine chooses tool and value. Fully reproducible.
    D0,
    /// Model chooses the tool from declared options. Engine computes values.
    D1,
    /// Model chooses tool and value. Requires a stateful envelope.
    D2,
}

impl AutonomyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyLevel::D0 => "D0",
            AutonomyLevel::D1 => "D1",
            AutonomyLevel::D2 => "D2",
        }
    }

    pub fn selected_by(self) -> SelectedBy {
        match self {
            AutonomyLevel::D0 => SelectedBy::Engine,
            AutonomyLevel::D1 | AutonomyLevel::D2 => SelectedBy::Model,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("D2 requires a declaration with stateful invariants or rate limits; this one has neither")]
    StatefulEnvelopeRequired,
    #[error("autonomy level {0} needs a model backend; construct the selector directly with one")]
    NoModelBackend(&'static str),
    #[error("no candidate tool can move '{0}'")]
    NoCandidate(String),
    #[error("model selected '{0}', which is not among the declared candidates")]
    UndeclaredSelection(String),
    #[error("model returned a malformed selection: {0}")]
    MalformedSelection(String),
    #[error(transparent)]
    Core(#[from] adi_core::CoreError),
}

/// What a selector returns.
#[derive(Debug, Clone)]
pub struct Selection {
    pub tool: SemanticId,
    pub parameter: String,
    /// Set only at D2, where the model supplies the value directly.
    pub model_value: Option<Quantity>,
}

/// The one seam between deterministic and model-driven behaviour.
pub trait ActionSelector {
    fn level(&self) -> AutonomyLevel;

    /// Choose from the candidates the planner derived from declaration and
    /// skill. The candidate list is the safety boundary: a selector can only
    /// ever return something the machine actually declared.
    fn select(
        &self,
        diagnosis: &Diagnosis,
        candidates: &[Candidate],
    ) -> Result<Selection, DecisionError>;
}

/// D0. Takes the highest-confidence candidate.
pub struct EngineSelector;

impl ActionSelector for EngineSelector {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D0
    }

    fn select(
        &self,
        diagnosis: &Diagnosis,
        candidates: &[Candidate],
    ) -> Result<Selection, DecisionError> {
        let first = candidates.first().ok_or_else(|| {
            DecisionError::NoCandidate(
                diagnosis
                    .target_variable
                    .as_ref()
                    .map(|v| v.0.clone())
                    .unwrap_or_else(|| "unspecified".into()),
            )
        })?;
        Ok(Selection {
            tool: first.tool.clone(),
            parameter: first.parameter.clone(),
            model_value: None,
        })
    }
}

/// Anything that can answer a closed-schema question. Implemented over a local
/// model in the shipping build, and over a fixture in tests.
pub trait ToolChooser {
    /// Given a diagnosis and the declared candidates, return the index of the
    /// chosen candidate. Returning an index rather than a name means the model
    /// cannot invent a tool that does not exist.
    fn choose(&self, diagnosis: &Diagnosis, candidates: &[Candidate]) -> Result<usize, String>;
}

/// D1. Model picks the tool, engine computes the value.
///
/// The model's output is an index into a list the engine built. It cannot name
/// an undeclared tool, and it never sees or produces a magnitude.
pub struct ModelToolSelector<C: ToolChooser> {
    pub chooser: C,
}

impl<C: ToolChooser> ActionSelector for ModelToolSelector<C> {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D1
    }

    fn select(
        &self,
        diagnosis: &Diagnosis,
        candidates: &[Candidate],
    ) -> Result<Selection, DecisionError> {
        if candidates.is_empty() {
            return Err(DecisionError::NoCandidate(
                diagnosis
                    .target_variable
                    .as_ref()
                    .map(|v| v.0.clone())
                    .unwrap_or_else(|| "unspecified".into()),
            ));
        }
        let index = self
            .chooser
            .choose(diagnosis, candidates)
            .map_err(DecisionError::MalformedSelection)?;
        let chosen = candidates
            .get(index)
            .ok_or_else(|| DecisionError::UndeclaredSelection(index.to_string()))?;
        Ok(Selection {
            tool: chosen.tool.clone(),
            parameter: chosen.parameter.clone(),
            model_value: None,
        })
    }
}

/// Anything that can choose both a tool and a magnitude.
pub trait ActionChooser {
    fn choose(
        &self,
        diagnosis: &Diagnosis,
        candidates: &[Candidate],
    ) -> Result<(usize, f64), String>;
}

/// D2. Model picks the tool and the value.
///
/// Construct with [`ModelActionSelector::new`], which refuses a declaration
/// without a stateful envelope.
pub struct ModelActionSelector<C: ActionChooser> {
    chooser: C,
}

impl<C: ActionChooser> ModelActionSelector<C> {
    pub fn new(chooser: C, declaration: &Declaration) -> Result<Self, DecisionError> {
        if !declaration.has_stateful_envelope() {
            return Err(DecisionError::StatefulEnvelopeRequired);
        }
        Ok(Self { chooser })
    }
}

impl<C: ActionChooser> ActionSelector for ModelActionSelector<C> {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D2
    }

    fn select(
        &self,
        diagnosis: &Diagnosis,
        candidates: &[Candidate],
    ) -> Result<Selection, DecisionError> {
        if candidates.is_empty() {
            return Err(DecisionError::NoCandidate(
                diagnosis
                    .target_variable
                    .as_ref()
                    .map(|v| v.0.clone())
                    .unwrap_or_else(|| "unspecified".into()),
            ));
        }
        let (index, value) = self
            .chooser
            .choose(diagnosis, candidates)
            .map_err(DecisionError::MalformedSelection)?;
        let chosen = candidates
            .get(index)
            .ok_or_else(|| DecisionError::UndeclaredSelection(index.to_string()))?;
        Ok(Selection {
            tool: chosen.tool.clone(),
            parameter: chosen.parameter.clone(),
            // Still clamped by the planner and checked by adi-safety. A model
            // choosing the number does not mean the number escapes the
            // envelope.
            model_value: Some(Quantity::new(value, adi_protocol::units::Unit::None)),
        })
    }
}

/// Build a selector for a level that needs no model backend.
///
/// Only D0 can be constructed this way, because D1 and D2 require a
/// [`ToolChooser`] or [`ActionChooser`]. Build those directly with
/// [`ModelToolSelector`] or [`ModelActionSelector::new`], which is also where
/// D2's stateful-envelope check happens.
pub fn selector_for(
    level: AutonomyLevel,
    declaration: &Declaration,
) -> Result<Box<dyn ActionSelector>, DecisionError> {
    match level {
        AutonomyLevel::D0 => Ok(Box::new(EngineSelector)),
        AutonomyLevel::D1 => Err(DecisionError::NoModelBackend("D1")),
        AutonomyLevel::D2 => {
            // Report the harder failure first: without a stateful envelope this
            // level is refused regardless of whether a backend is available.
            if !declaration.has_stateful_envelope() {
                return Err(DecisionError::StatefulEnvelopeRequired);
            }
            Err(DecisionError::NoModelBackend("D2"))
        }
    }
}

/// Convenience: derive candidates, select, and plan the magnitude.
pub fn decide(
    planner: &Planner<'_>,
    selector: &dyn ActionSelector,
    diagnosis: &Diagnosis,
    current: Quantity,
) -> Result<adi_core::plan::Correction, DecisionError> {
    let variable = diagnosis
        .target_variable
        .as_ref()
        .ok_or_else(|| DecisionError::NoCandidate("unspecified".into()))?;
    let candidates = planner.candidates(variable);
    let selection = selector.select(diagnosis, &candidates)?;
    let chosen = candidates
        .iter()
        .find(|c| c.tool == selection.tool && c.parameter == selection.parameter)
        .ok_or_else(|| DecisionError::UndeclaredSelection(selection.tool.0.clone()))?;
    Ok(planner.plan(diagnosis, chosen, current)?)
}

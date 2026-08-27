//! A cycle: one bounded run of a process that produces measurable outcomes.
//!
//! In one domain a cycle is a coffee extraction. In another it is a wash, a
//! print, a bake, or a week of growth. This module must stay ignorant of which.

use std::collections::BTreeMap;

use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::units::Quantity;

/// A recorded time series from one signal.
#[derive(Debug, Clone)]
pub struct Trace {
    pub signal: SemanticId,
    /// (seconds since cycle start, value)
    pub samples: Vec<(f64, f64)>,
    pub unit: adi_protocol::units::Unit,
    /// Declared accuracy, carried through so downstream code can tell a real
    /// difference from noise.
    pub accuracy_abs: f64,
    /// True if the appliance already smoothed this, so the engine does not
    /// filter twice.
    pub filtered: bool,
}

impl Trace {
    pub fn duration_s(&self) -> f64 {
        match (self.samples.first(), self.samples.last()) {
            (Some(first), Some(last)) => last.0 - first.0,
            _ => 0.0,
        }
    }

    pub fn final_value(&self) -> Option<f64> {
        self.samples.last().map(|s| s.1)
    }

    /// True when the difference exceeds the declared measurement accuracy.
    pub fn is_significant(&self, delta: f64) -> bool {
        delta.abs() > self.accuracy_abs
    }
}

/// What the operator said about the result. Subjective, optional, and the only
/// input the engine cannot measure for itself.
#[derive(Debug, Clone, Default)]
pub struct SubjectiveFeedback {
    /// Domain-defined descriptors, for example "bitter" or "still stained".
    pub descriptors: Vec<String>,
    /// Optional 0 to 5 rating.
    pub rating: Option<u8>,
    /// Free text, for the language layer only. Never an actuation input.
    pub note: Option<String>,
}

/// One completed cycle.
#[derive(Debug, Clone)]
pub struct Cycle {
    pub cycle_id: String,
    pub subject_id: Option<String>,
    pub operator_id: Option<String>,
    /// Settings in force for this cycle.
    pub settings: BTreeMap<VariableId, Quantity>,
    /// Measured outcomes.
    pub measured: BTreeMap<VariableId, Quantity>,
    pub traces: Vec<Trace>,
    pub feedback: SubjectiveFeedback,
}

impl Cycle {
    pub fn trace(&self, signal: &SemanticId) -> Option<&Trace> {
        self.traces.iter().find(|t| &t.signal == signal)
    }

    pub fn value(&self, variable: &VariableId) -> Option<Quantity> {
        self.measured
            .get(variable)
            .or_else(|| self.settings.get(variable))
            .copied()
    }
}

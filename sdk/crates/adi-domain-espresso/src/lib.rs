//! The espresso domain profile.
//!
//! This is the only crate in the SDK permitted to know what coffee is.

pub mod registry;
pub mod rules;

use adi_core::cycle::Cycle;
use adi_core::domain::{Diagnosis, Domain};
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::ResolvedTargets;
use adi_protocol::units::{Range, Unit};

pub struct Espresso {
    variables: Vec<VariableId>,
}

impl Default for Espresso {
    fn default() -> Self {
        Self::new()
    }
}

impl Espresso {
    pub fn new() -> Self {
        Self {
            variables: [
                registry::vars::DOSE_MASS,
                registry::vars::YIELD_MASS,
                registry::vars::RATIO,
                registry::vars::SHOT_TIME,
                registry::vars::GRIND_SETTING,
                registry::vars::BREW_TEMPERATURE,
                registry::vars::EXTRACTION_YIELD,
                registry::vars::FLOW_PEAK,
                registry::vars::FLOW_VARIANCE,
            ]
            .iter()
            .map(|v| VariableId::from(*v))
            .collect(),
        }
    }
}

impl Domain for Espresso {
    fn profile_id(&self) -> &str {
        registry::PROFILE_ID
    }

    fn variables(&self) -> &[VariableId] {
        &self.variables
    }

    fn base_targets(&self, style: &str) -> ResolvedTargets {
        let mut targets = ResolvedTargets::new();
        let layer = adi_protocol::knowledge::Layer::Base;

        // Loaded from profiles/espresso/knowledge.yaml in the real build.
        // Inlined here so the scaffold has something to reason over.
        let (ratio, time) = match style {
            "ristretto" => ((1.2, 1.8), (22.0, 30.0)),
            "lungo" => ((3.0, 4.0), (28.0, 40.0)),
            _ => ((1.8, 2.4), (25.0, 32.0)),
        };

        let _ = targets.apply(
            registry::vars::RATIO,
            Range {
                min: ratio.0,
                max: ratio.1,
                resolution: None,
                unit: Unit::None,
            },
            layer,
        );
        let _ = targets.apply(
            registry::vars::SHOT_TIME,
            Range {
                min: time.0,
                max: time.1,
                resolution: None,
                unit: Unit::S,
            },
            layer,
        );
        let _ = targets.apply(
            registry::vars::BREW_TEMPERATURE,
            Range {
                min: 90.0,
                max: 96.0,
                resolution: Some(0.5),
                unit: Unit::C,
            },
            layer,
        );
        targets
    }

    fn derive(&self, cycle: &mut Cycle) {
        rules::derive(cycle);
    }

    fn diagnose(&self, cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis {
        rules::diagnose(cycle, targets)
    }

    fn default_step(&self, variable: &VariableId) -> Option<f64> {
        match variable.as_str() {
            registry::vars::SHOT_TIME => Some(2.0),
            registry::vars::RATIO => Some(0.1),
            registry::vars::BREW_TEMPERATURE => Some(0.5),
            _ => None,
        }
    }

    fn feedback_vocabulary(&self) -> &[&str] {
        registry::FEEDBACK_VOCABULARY
    }
}

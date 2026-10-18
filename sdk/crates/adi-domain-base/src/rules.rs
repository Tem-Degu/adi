//! The generic correction.
//!
//! One rule, and it is the only rule that holds without knowing what the
//! machine is: a measured outcome sitting outside its target gets moved back
//! towards that target.
//!
//! What this rule cannot do is say why. A profile that knows its domain rules
//! out the causes that an adjustment would make worse, and checks them first:
//! in espresso, erratic flow is a tamping fault, and prescribing a grind
//! change for it ruins the next cup. This rule has no such knowledge and does
//! not pretend to. It reports that something drifted, corrects cautiously, and
//! leaves the diagnosis to a profile that has one.

use adi_core::cycle::Cycle;
use adi_core::domain::Diagnosis;
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{Knowledge, ResolvedTargets};

/// How sure a correction from this rule is.
///
/// Lower than any profile rule on purpose. The engine sizes a correction by
/// confidence, so a cautious number arrives at the target over two or three
/// cycles instead of one. Overshooting on a guess is the failure to avoid
/// here, because the rule knows a value drifted and not why it drifted.
const CONFIDENCE: f64 = 0.5;

/// The measured outcome furthest outside its target, and how far out it is.
///
/// Distance is measured against the width of the target, not in the variable's
/// own unit, because the units are not comparable: two degrees and two grams
/// cannot be ranked against each other, while a tenth of a target and half a
/// target can.
fn furthest_outside<'a>(
    cycle: &'a Cycle,
    targets: &'a ResolvedTargets,
    knowledge: &Knowledge,
) -> Option<(&'a VariableId, f64, f64, f64)> {
    let mut worst: Option<(&VariableId, f64, f64, f64)> = None;
    let mut worst_relative = 0.0_f64;

    // `measured` is ordered, so a tie resolves the same way on every run and
    // a decision stays reproducible.
    for (variable, quantity) in &cycle.measured {
        // ADI-P 7.1: a preference records what someone asked for and never how
        // the machine performed. Correcting one would be overriding the
        // person, so the engine must never do it to fix a fault.
        if knowledge.is_preference(variable.0.as_str()) {
            continue;
        }

        let Some(target) = targets.get(variable.0.as_str()) else {
            continue;
        };
        let range = &target.value;

        // Comparing a value in one unit against a target in another is
        // meaningless. Skipped rather than converted, because a profile
        // declaring a target in the wrong unit is a fault to find and not one
        // to paper over.
        if range.unit != quantity.unit {
            continue;
        }
        if range.contains(quantity.value) {
            continue;
        }

        let outside = if quantity.value < range.min {
            range.min - quantity.value
        } else {
            quantity.value - range.max
        };
        let width = (range.max - range.min).abs();
        let relative = if width > f64::EPSILON {
            outside / width
        } else {
            outside
        };

        if relative > worst_relative {
            worst_relative = relative;
            worst = Some((variable, quantity.value, range.min, range.max));
        }
    }

    worst
}

pub fn diagnose(cycle: &Cycle, targets: &ResolvedTargets, knowledge: &Knowledge) -> Diagnosis {
    match furthest_outside(cycle, targets, knowledge) {
        Some((variable, measured, min, max)) => {
            // The middle of the target, so a correction lands inside it rather
            // than on its edge, where the next ordinary variation puts it back
            // outside.
            let delta = (min + max) / 2.0 - measured;
            Diagnosis {
                cause: "out_of_target".into(),
                target_variable: Some(variable.clone()),
                desired_delta: Some(delta),
                confidence: CONFIDENCE,
                explanation: format!(
                    "{} measured {:.3} against a target of {:.3} to {:.3}. \
                     Correcting towards the middle of that range. This profile \
                     reports that it drifted and cannot say why.",
                    variable.0, measured, min, max
                ),
            }
        }
        // Two different situations, and telling them apart matters to whoever
        // reads the record. Nothing measured usually means the appliance sent
        // raw signals and this profile derives nothing from them.
        None if cycle.measured.is_empty() => Diagnosis::advisory(
            "nothing_measured",
            "This cycle reported no measured outcome, so there was nothing to \
             compare against a target.",
        ),
        None => Diagnosis::advisory(
            "within_target",
            "Everything measured landed inside its target.",
        ),
    }
}

//! Espresso diagnosis rules.
//!
//! Rule ordering is deliberate and is the most important thing in this file.
//! Channeling is a preparation fault. Prescribing a grind change for it makes
//! the next shot worse, so it must be ruled out before anything else is
//! considered. This ordering is why `Domain::diagnose` is code rather than a
//! data table.

use adi_core::cycle::{Cycle, Trace};
use adi_core::domain::Diagnosis;
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::knowledge::ResolvedTargets;
use adi_protocol::units::{Quantity, Unit};

use crate::registry::{events, signals, vars};

/// Turn raw traces into modelled variables.
pub fn derive(cycle: &mut Cycle) {
    let mass_signal = SemanticId::from(signals::MASS);
    let Some(trace) = cycle.trace(&mass_signal) else {
        return;
    };

    let yield_mass = trace.final_value().unwrap_or(0.0);
    let duration = shot_time(cycle, trace);
    let flow = flow_series(&trace.samples);
    let peak = flow.iter().map(|(_, f)| *f).fold(0.0_f64, f64::max);
    let variance = flow_variance(&flow);

    cycle.measured.insert(
        VariableId::from(vars::YIELD_MASS),
        Quantity::new(yield_mass, Unit::G),
    );
    cycle.measured.insert(
        VariableId::from(vars::SHOT_TIME),
        Quantity::new(duration, Unit::S),
    );
    cycle.measured.insert(
        VariableId::from(vars::FLOW_PEAK),
        Quantity::new(peak, Unit::GPerS),
    );
    cycle.measured.insert(
        VariableId::from(vars::FLOW_VARIANCE),
        Quantity::new(variance, Unit::None),
    );

    if let Some(dose) = cycle.settings.get(&VariableId::from(vars::DOSE_MASS)) {
        if dose.value > 0.0 {
            cycle.measured.insert(
                VariableId::from(vars::RATIO),
                Quantity::new(yield_mass / dose.value, Unit::None),
            );
        }
    }
}

/// Shot time: first drip to stop. ADI-P 7.3.
///
/// Not the span of the trace, which is what this measured until now. A shot
/// spends its first seconds saturating the puck with nothing in the cup, and
/// counting that time makes every shot look longer than it ran. Several
/// seconds against a 25 to 32 second target is not a rounding error: it biases
/// every correction, and in the wrong direction, because a shot that looks
/// slow gets a coarser grind when it needed none.
///
/// The appliance is asked first. `adi.espresso.event.first_drip` exists in the
/// registry precisely because the machine can see flow begin and a mass curve
/// can only be used to guess at it.
fn shot_time(cycle: &Cycle, trace: &Trace) -> f64 {
    let end = trace.samples.last().map(|(t, _)| *t).unwrap_or(0.0);
    let start = first_drip(cycle, trace);
    (end - start).max(0.0)
}

/// When flow began, from the appliance if it said and from the curve if not.
fn first_drip(cycle: &Cycle, trace: &Trace) -> f64 {
    let declared = SemanticId::from(events::FIRST_DRIP);
    if let Some(event) = cycle.events.iter().find(|e| e.id == declared) {
        return event.at_s;
    }

    // Detected: the first sample that rises above what the scale can resolve.
    // This is what `accuracy` is mandatory for, and it is why guessing is only
    // the fallback: at 20 Hz it is close, and on a coarse trace it is not.
    trace
        .samples
        .iter()
        .find(|(_, mass)| *mass > trace.accuracy_abs)
        .map(|(t, _)| *t)
        .unwrap_or(0.0)
}

/// First difference of the mass series. Real builds smooth this first: the
/// derivative of a noisy weight signal is noisier still, and drips landing on
/// the scale produce spikes that are not flow.
fn flow_series(samples: &[(f64, f64)]) -> Vec<(f64, f64)> {
    samples
        .windows(2)
        .filter_map(|w| {
            let dt = w[1].0 - w[0].0;
            if dt <= 0.0 {
                return None;
            }
            Some((w[1].0, (w[1].1 - w[0].1) / dt))
        })
        .collect()
}

/// Coefficient of variation over the stable portion of the shot.
///
/// Placeholder. The real detector is phase 8 of the roadmap and needs labelled
/// data from filmed shots. Treat this number as a starting point, not a
/// detector.
fn flow_variance(flow: &[(f64, f64)]) -> f64 {
    let stable: Vec<f64> = flow
        .iter()
        .filter(|(t, _)| *t > 5.0)
        .map(|(_, f)| *f)
        .collect();
    if stable.len() < 3 {
        return 0.0;
    }
    let mean = stable.iter().sum::<f64>() / stable.len() as f64;
    if mean.abs() < f64::EPSILON {
        return 0.0;
    }
    let var = stable.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / stable.len() as f64;
    var.sqrt() / mean
}

const CHANNELING_VARIANCE_THRESHOLD: f64 = 0.45;

pub fn diagnose(cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis {
    let shot_time = cycle
        .value(&VariableId::from(vars::SHOT_TIME))
        .map(|q| q.value)
        .unwrap_or(0.0);
    let variance = cycle
        .value(&VariableId::from(vars::FLOW_VARIANCE))
        .map(|q| q.value)
        .unwrap_or(0.0);

    let time_target = targets.get(vars::SHOT_TIME);
    let (t_min, t_max) = match time_target {
        Some(t) => (t.value.min, t.value.max),
        None => (25.0, 32.0),
    };

    let says = |d: &str| cycle.feedback.descriptors.iter().any(|x| x == d);

    // 1. Preparation faults first. Erratic flow with contradictory taste is
    //    channeling, and a grind change makes it worse.
    if variance > CHANNELING_VARIANCE_THRESHOLD && (says("sour") || says("bitter")) {
        return Diagnosis {
            cause: "channeling".into(),
            target_variable: None,
            desired_delta: None,
            confidence: 0.6,
            explanation: "The flow was erratic, which points at channeling rather than \
                          grind. Distribute more evenly and tamp level, then pull again."
                .into(),
        };
    }

    // 2. Under-extraction.
    if says("sour") || shot_time < t_min {
        let delta = (t_min + t_max) / 2.0 - shot_time;
        return Diagnosis {
            cause: "under_extracted".into(),
            target_variable: Some(VariableId::from(vars::SHOT_TIME)),
            desired_delta: Some(delta),
            confidence: if says("sour") { 0.8 } else { 0.6 },
            explanation: format!(
                "That ran {shot_time:.0} seconds against a target of {t_min:.0} to \
                 {t_max:.0}. It is under-extracted, so slow the flow down."
            ),
        };
    }

    // 3. Over-extraction.
    if says("bitter") || shot_time > t_max {
        let delta = (t_min + t_max) / 2.0 - shot_time;
        return Diagnosis {
            cause: "over_extracted".into(),
            target_variable: Some(VariableId::from(vars::SHOT_TIME)),
            desired_delta: Some(delta),
            confidence: if says("bitter") { 0.8 } else { 0.6 },
            explanation: format!(
                "That ran {shot_time:.0} seconds against a target of {t_min:.0} to \
                 {t_max:.0}. It is over-extracted, so speed the flow up."
            ),
        };
    }

    Diagnosis::advisory("balanced", "That one landed in the target. Save it as a recipe?")
}

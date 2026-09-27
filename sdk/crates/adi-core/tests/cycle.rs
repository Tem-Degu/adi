//! Cycles and traces.
//!
//! A cycle is one bounded run of a process. In one domain that is an
//! extraction, in another a wash, a bake, or a week of growth. Nothing here may
//! know which.

use adi_core::{Cycle, SubjectiveFeedback, Trace};
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::units::{Quantity, Unit};
use std::collections::BTreeMap;

fn trace(samples: &[(f64, f64)], accuracy_abs: f64) -> Trace {
    Trace {
        signal: SemanticId("adi.example.signal.mass".into()),
        samples: samples.to_vec(),
        unit: Unit::G,
        accuracy_abs,
        filtered: false,
    }
}

fn cycle() -> Cycle {
    Cycle {
        cycle_id: "c1".into(),
        subject_id: None,
        operator_id: None,
        settings: BTreeMap::new(),
        measured: BTreeMap::new(),
        traces: vec![trace(&[(0.0, 0.0), (10.0, 12.0), (25.0, 30.0)], 0.2)],
        events: Vec::new(),
        feedback: SubjectiveFeedback::default(),
    }
}

/// What was measured beats what was asked for. A machine told to stop at 30
/// that actually delivered 28 must be reasoned about as 28, or every correction
/// is computed from a number that never happened.
#[test]
fn a_measured_value_wins_over_the_setting_that_asked_for_it() {
    let mut c = cycle();
    let v = VariableId::from("output_volume");
    c.settings.insert(v.clone(), Quantity::new(30.0, Unit::Ml));
    assert_eq!(c.value(&v).unwrap().value, 30.0);

    c.measured.insert(v.clone(), Quantity::new(28.0, Unit::Ml));
    assert_eq!(c.value(&v).unwrap().value, 28.0);
}

#[test]
fn an_absent_variable_is_none_rather_than_zero() {
    assert!(cycle().value(&VariableId::from("never_set")).is_none());
}

/// Declared accuracy is what separates a real change from sensor noise. Without
/// it the engine chases its own measurement error, correcting every cycle and
/// converging on nothing.
#[test]
fn a_difference_within_the_declared_accuracy_is_not_significant() {
    let t = trace(&[(0.0, 0.0)], 0.2);
    assert!(!t.is_significant(0.1));
    assert!(!t.is_significant(-0.1));
    assert!(t.is_significant(0.3));
    assert!(t.is_significant(-0.3), "significance is about magnitude");
    assert!(
        !t.is_significant(0.2),
        "exactly at the declared accuracy is not yet a difference"
    );
}

#[test]
fn a_trace_reports_its_span_and_final_value() {
    let t = trace(&[(2.0, 0.0), (10.0, 12.0), (25.0, 30.0)], 0.2);
    assert_eq!(t.duration_s(), 23.0, "measured from the first sample, not zero");
    assert_eq!(t.final_value(), Some(30.0));
}

/// An empty trace must not produce a duration or a value out of nothing.
#[test]
fn an_empty_trace_reports_nothing() {
    let t = trace(&[], 0.2);
    assert_eq!(t.duration_s(), 0.0);
    assert_eq!(t.final_value(), None);
}

#[test]
fn a_trace_is_found_by_its_signal_id() {
    let c = cycle();
    assert!(c
        .trace(&SemanticId("adi.example.signal.mass".into()))
        .is_some());
    assert!(c
        .trace(&SemanticId("adi.example.signal.elapsed".into()))
        .is_none());
}

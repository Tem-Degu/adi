//! Decision records. ADI-P 9.3 requirement 5.
//!
//! A decision must be reconstructable: the inputs, the layer behind each value,
//! and the rule that produced it. This is what answers the question a regulator
//! or an insurer eventually asks, which is why the machine did that.

use adi_core::{DecisionRecord, SelectedBy, ValueProvenance, ValueSource};
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::knowledge::Layer;
use adi_protocol::units::{Quantity, Unit};
use std::collections::BTreeMap;

fn record(selected_by: SelectedBy) -> DecisionRecord {
    let tool = SemanticId("adi.example.actuator.set_position".into());
    DecisionRecord {
        cycle_id: "c1".into(),
        decision_id: "d1".into(),
        autonomy_level: "D1",
        cause: "out_of_target".into(),
        rule_id: "rule.example.slow".into(),
        selected_tool: Some(tool.clone()),
        selected_by,
        inputs: vec![
            ValueProvenance {
                variable: VariableId::from("cycle_time"),
                value: Quantity::new(22.0, Unit::S),
                layer: Layer::Base,
                source: ValueSource::Measured {
                    signal: SemanticId("adi.example.signal.elapsed".into()),
                },
            },
            ValueProvenance {
                variable: VariableId::from("input_mass"),
                value: Quantity::new(9.0, Unit::G),
                layer: Layer::Vendor,
                source: ValueSource::Knowledge {
                    key: "input_mass".into(),
                },
            },
            ValueProvenance {
                variable: VariableId::from("position"),
                value: Quantity::new(-2.0, Unit::Step),
                layer: Layer::Base,
                source: ValueSource::SkillResponse {
                    tool,
                    parameter: "position".into(),
                },
            },
        ],
        arguments: BTreeMap::from([("position".to_string(), Quantity::new(28.0, Unit::Step))]),
        clamped_from: BTreeMap::new(),
        explanation: "ran short of the target".into(),
    }
}

/// A model-selected tool makes the selection non-reproducible even though every
/// value remains deterministic. That is the honest cost of D1, and the record
/// must state it rather than imply a replay that would not match.
#[test]
fn a_model_selected_decision_is_not_reproducible() {
    assert!(record(SelectedBy::Engine).is_reproducible());
    assert!(!record(SelectedBy::Model).is_reproducible());
}

/// The example from 9.3: "the dose came from the vendor default, the
/// temperature from the user's learned preference" is the level of detail
/// required, so every input has to carry its own layer rather than the record
/// carrying one layer overall.
#[test]
fn each_input_carries_its_own_layer() {
    let r = record(SelectedBy::Engine);
    let layers: Vec<Layer> = r.inputs.iter().map(|i| i.layer).collect();
    assert!(
        layers.contains(&Layer::Vendor) && layers.contains(&Layer::Base),
        "a single decision draws on more than one layer at once"
    );
}

/// Measured, knowledge and skill-derived values are different kinds of thing.
/// Collapsing them would lose the distinction between what the machine saw and
/// what it was told.
#[test]
fn the_source_of_each_value_is_distinguishable() {
    let r = record(SelectedBy::Engine);
    let measured = r
        .inputs
        .iter()
        .filter(|i| matches!(i.source, ValueSource::Measured { .. }))
        .count();
    let from_skill = r
        .inputs
        .iter()
        .filter(|i| matches!(i.source, ValueSource::SkillResponse { .. }))
        .count();
    assert_eq!((measured, from_skill), (1, 1));
    assert_ne!(ValueSource::ProfileDefault, ValueSource::Operator);
}

/// Clamping is recorded separately from the argument sent, because "asked for
/// 65, sent 60" and "asked for 60" are different decisions and only one of them
/// says the machine ran out of range.
#[test]
fn clamping_is_recorded_apart_from_the_value_sent() {
    let mut r = record(SelectedBy::Engine);
    assert!(r.clamped_from.is_empty());

    r.clamped_from.insert("position".into(), 65.0);
    r.arguments
        .insert("position".into(), Quantity::new(60.0, Unit::Step));

    assert_eq!(r.clamped_from["position"], 65.0);
    assert_eq!(r.arguments["position"].value, 60.0);
}

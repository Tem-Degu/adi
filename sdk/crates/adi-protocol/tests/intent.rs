//! The intent schema. ADI-P 9.3: the only thing a model may emit on the
//! request path, and the thing that decides how expressive the product can be.

use adi_protocol::intent::{Intent, IntentError, IntentKind, Reference, TargetSpec};
use adi_protocol::units::Unit;
use adi_protocol::VariableId;

fn parse(yaml: &str) -> Intent {
    serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("intent should parse: {e}"))
}

/// The shape a manufacturer or integrator writes by hand, and the shape a model
/// must emit. If this stops being readable, the schema is wrong.
#[test]
fn the_written_form_parses() {
    let intent = parse(
        r#"
kind: adjust
style: "adi.example.style.standard"
targets:
  additive_volume: { of: last, scale: 0.5 }
  cycle_time:      { value: 30, unit: s }
"#,
    );

    assert_eq!(intent.kind, IntentKind::Adjust);
    assert_eq!(intent.style.as_deref(), Some("adi.example.style.standard"));

    match &intent.targets[&VariableId::from("additive_volume")] {
        TargetSpec::Relative(r) => {
            assert_eq!(r.of, Reference::Last);
            assert_eq!(r.scale, Some(0.5));
            assert!(r.offset.is_none());
        }
        other => panic!("expected a relative target, got {other:?}"),
    }

    match &intent.targets[&VariableId::from("cycle_time")] {
        TargetSpec::Absolute(q) => {
            assert_eq!((q.value, q.unit), (30.0, Unit::S));
        }
        other => panic!("expected an absolute target, got {other:?}"),
    }

    intent.validate().expect("this intent is well formed");
}

/// There is no way to name a tool in this schema. That is the point: a model
/// emitting intent cannot ask for an action, only for an outcome.
#[test]
fn the_schema_has_no_place_to_name_a_tool() {
    let text = serde_yaml::to_string(&parse(
        r#"
kind: produce
style: "adi.example.style.standard"
targets:
  cycle_time: { value: 30, unit: s }
"#,
    ))
    .unwrap();
    assert!(!text.contains("tool"));
    assert!(!text.contains("parameter"));
}

#[test]
fn an_offset_form_parses() {
    let intent = parse(
        r#"
kind: adjust
targets:
  additive_volume: { of: last, offset: { value: 2, unit: ml } }
"#,
    );
    match &intent.targets[&VariableId::from("additive_volume")] {
        TargetSpec::Relative(r) => {
            assert_eq!(r.offset.map(|q| (q.value, q.unit)), Some((2.0, Unit::Ml)));
            assert!(r.scale.is_none());
        }
        other => panic!("expected a relative target, got {other:?}"),
    }
    intent.validate().unwrap();
}

#[test]
fn a_specific_cycle_can_be_referenced() {
    let intent = parse(
        r#"
kind: adjust
targets:
  additive_volume: { of: "2026-08-25T09:14", scale: 0.5 }
"#,
    );
    match &intent.targets[&VariableId::from("additive_volume")] {
        TargetSpec::Relative(r) => {
            assert_eq!(r.of, Reference::Cycle("2026-08-25T09:14".into()))
        }
        other => panic!("expected a relative target, got {other:?}"),
    }
}

// ------------------------------------------------------------- validation

#[test]
fn an_adjustment_naming_nothing_is_refused() {
    assert!(matches!(
        parse("kind: adjust").validate(),
        Err(IntentError::NoTargets)
    ));
}

/// "Produce" with no targets is meaningful: make the usual thing.
#[test]
fn producing_with_no_targets_is_allowed() {
    parse("kind: produce\nstyle: \"adi.example.style.standard\"")
        .validate()
        .expect("make the usual one");
}

#[test]
fn a_target_giving_both_a_scale_and_an_offset_is_refused() {
    let intent = parse(
        r#"
kind: adjust
targets:
  additive_volume: { of: last, scale: 0.5, offset: { value: 2, unit: ml } }
"#,
    );
    assert!(matches!(
        intent.validate(),
        Err(IntentError::ScaleAndOffset(_))
    ));
}

/// Zero is a real request: none at all.
#[test]
fn a_scale_of_zero_is_allowed_but_a_negative_one_is_not() {
    parse("kind: adjust\ntargets:\n  additive_volume: { of: last, scale: 0.0 }")
        .validate()
        .expect("none at all is a thing people ask for");

    assert!(matches!(
        parse("kind: adjust\ntargets:\n  additive_volume: { of: last, scale: -1.0 }").validate(),
        Err(IntentError::NegativeScale(_))
    ));
}

/// A non-finite number would survive into a magnitude if it were not stopped
/// here, and the planner is the last gate rather than the only one.
#[test]
fn a_non_finite_number_is_refused() {
    assert!(matches!(
        parse("kind: adjust\ntargets:\n  additive_volume: { of: last, scale: .inf }").validate(),
        Err(IntentError::NotFinite { .. })
    ));
    assert!(matches!(
        parse("kind: adjust\ntargets:\n  additive_volume: { value: .nan, unit: ml }").validate(),
        Err(IntentError::NotFinite { .. })
    ));
}

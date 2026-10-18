//! The base profile: what a machine gets before anyone writes it a domain.

use std::collections::BTreeMap;

use adi_core::cycle::{Cycle, SubjectiveFeedback};
use adi_core::domain::Domain;
use adi_domain_base::Base;
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{KnowledgeExtension, Layer, ResolvedTargets};
use adi_protocol::units::{Quantity, Range, Unit};

fn cycle(measured: &[(&str, f64, Unit)]) -> Cycle {
    let mut map = BTreeMap::new();
    for (name, value, unit) in measured {
        map.insert(VariableId::from(*name), Quantity::new(*value, *unit));
    }
    Cycle {
        cycle_id: "c1".into(),
        subject_id: None,
        operator_id: None,
        settings: BTreeMap::new(),
        measured: map,
        traces: Vec::new(),
        events: Vec::new(),
        feedback: SubjectiveFeedback {
            descriptors: Vec::new(),
            rating: None,
            note: None,
        },
    }
}

fn targets(entries: &[(&str, f64, f64, Unit)]) -> ResolvedTargets {
    let mut t = ResolvedTargets::new();
    for (name, min, max, unit) in entries {
        t.apply(
            name,
            Range {
                min: *min,
                max: *max,
                resolution: None,
                unit: *unit,
            },
            Layer::Base,
        )
        .expect("a first layer always applies");
    }
    t
}

const EXTENSION: &str = r#"
namespace: "x.acme"
extension_version: "1.0"
extends: { domain_profile: "base/v1", base_min_version: "1.0" }
variables:
  x.acme.rinse_temp:
    unit: c
    description: >
      Final rinse temperature. Higher dries better and risks warping plastics.
notes:
  - id: "x.acme.note.filter"
    text: "The filter unclips anticlockwise."
"#;

#[test]
fn it_starts_knowing_nothing() {
    let base = Base::new();
    assert_eq!(base.profile_id(), "base/v1");
    assert!(
        base.variables().is_empty(),
        "the base ships no knowledge; everything comes from the manufacturer"
    );
}

/// The whole point of the profile: a manufacturer's own file is the domain.
#[test]
fn a_manufacturer_supplies_the_domain() {
    let mut base = Base::new();
    let extension: KnowledgeExtension = serde_yaml::from_str(EXTENSION).unwrap();
    base.add_extension(extension, Layer::Vendor)
        .expect("a well formed extension on an empty base must layer");

    assert_eq!(
        base.variables(),
        &[VariableId::from("x.acme.rinse_temp")],
        "a variable the manufacturer declared must be reachable by language"
    );
}

/// ADI-P 6.5. A bad extension degrades a machine to standard behaviour and
/// never stops it working.
#[test]
fn a_rejected_extension_leaves_the_profile_as_it_was() {
    let mut base = Base::new();
    // Outside the declared namespace, which 6.2 refuses.
    let bad = EXTENSION.replace("x.acme.rinse_temp", "rinse_temp");
    let extension: KnowledgeExtension = serde_yaml::from_str(&bad).unwrap();

    assert!(base.add_extension(extension, Layer::Vendor).is_err());
    assert!(
        base.variables().is_empty(),
        "a refused extension must change nothing"
    );
}

#[test]
fn a_result_inside_its_target_needs_no_correction() {
    let base = Base::new();
    let d = base.diagnose(
        &cycle(&[("x.acme.rinse_temp", 50.0, Unit::C)]),
        &targets(&[("x.acme.rinse_temp", 45.0, 55.0, Unit::C)]),
    );
    assert_eq!(d.cause, "within_target");
    assert!(d.target_variable.is_none());
}

/// The one rule that holds without knowing what the machine is.
#[test]
fn a_result_outside_its_target_is_corrected_towards_the_middle() {
    let base = Base::new();
    let d = base.diagnose(
        &cycle(&[("x.acme.rinse_temp", 40.0, Unit::C)]),
        &targets(&[("x.acme.rinse_temp", 45.0, 55.0, Unit::C)]),
    );

    assert_eq!(d.cause, "out_of_target");
    assert_eq!(d.target_variable, Some(VariableId::from("x.acme.rinse_temp")));
    // The middle of 45 to 55 is 50, so 40 wants moving by 10.
    assert_eq!(d.desired_delta, Some(10.0));
    assert!(
        d.confidence < 0.8,
        "a correction that cannot say why must be less sure than one that can"
    );
}

/// Units are not comparable, so distance is measured against the width of each
/// target. Two degrees out of a two degree target is worse than ten grams out
/// of a hundred gram one.
#[test]
fn the_variable_furthest_outside_its_target_is_the_one_corrected() {
    let base = Base::new();
    let d = base.diagnose(
        &cycle(&[
            ("x.acme.coverage", 110.0, Unit::G),
            ("x.acme.rinse_temp", 57.0, Unit::C),
        ]),
        &targets(&[
            // Ten out of a hundred wide target: a tenth.
            ("x.acme.coverage", 0.0, 100.0, Unit::G),
            // Two out of a two wide target: a whole target.
            ("x.acme.rinse_temp", 53.0, 55.0, Unit::C),
        ]),
    );
    assert_eq!(d.target_variable, Some(VariableId::from("x.acme.rinse_temp")));
}

/// A target stated in a different unit from the measurement is a fault to
/// find, not one to guess past.
#[test]
fn a_target_in_another_unit_is_not_compared() {
    let base = Base::new();
    let d = base.diagnose(
        &cycle(&[("x.acme.rinse_temp", 40.0, Unit::C)]),
        &targets(&[("x.acme.rinse_temp", 45.0, 55.0, Unit::G)]),
    );
    assert_eq!(d.cause, "within_target");
}

/// An outcome with no target is not a fault. A machine measures plenty that
/// nobody stated a goal for.
#[test]
fn an_outcome_with_no_target_is_left_alone() {
    let base = Base::new();
    let d = base.diagnose(
        &cycle(&[("x.acme.rinse_temp", 40.0, Unit::C)]),
        &targets(&[]),
    );
    assert_eq!(d.cause, "within_target");
}

/// Said separately from "all good", because it usually means the appliance
/// sent raw curves and this profile derives nothing from them.
#[test]
fn measuring_nothing_is_reported_as_measuring_nothing() {
    let base = Base::new();
    let d = base.diagnose(&cycle(&[]), &targets(&[("x.acme.rinse_temp", 45.0, 55.0, Unit::C)]));
    assert_eq!(d.cause, "nothing_measured");
}

/// Perception belongs to a profile that has some. This one must not invent any.
#[test]
fn it_derives_nothing_from_raw_signals() {
    let base = Base::new();
    let mut c = cycle(&[]);
    base.derive(&mut c);
    assert!(c.measured.is_empty());
}

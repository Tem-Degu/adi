//! Diagnosis and correction planning for the espresso profile.
//!
//! The first test is the important one. Channeling is a preparation fault, and
//! prescribing a grind change for it makes the next cycle worse. Rule ordering
//! is the thing being tested, and it is why `Domain::diagnose` is code rather
//! than a lookup table.

use std::collections::BTreeMap;

use adi_core::cycle::{Cycle, SubjectiveFeedback, Trace};
use adi_core::domain::Domain;
use adi_core::plan::Planner;
use adi_protocol::ids::VariableId;
use adi_protocol::units::{Quantity, Unit};
use adi_protocol::{Declaration, SemanticId, Skill};
use adi_domain_espresso::registry::{signals, vars};
use adi_domain_espresso::Espresso;

fn cycle(samples: &[(f64, f64)], descriptors: &[&str], grind: f64) -> Cycle {
    let mut settings = BTreeMap::new();
    settings.insert(
        VariableId::from(vars::DOSE_MASS),
        Quantity::new(9.0, Unit::G),
    );
    settings.insert(
        VariableId::from(vars::GRIND_SETTING),
        Quantity::new(grind, Unit::Step),
    );

    Cycle {
        cycle_id: "t".into(),
        subject_id: None,
        operator_id: None,
        settings,
        measured: BTreeMap::new(),
        traces: vec![Trace {
            signal: SemanticId::from(signals::MASS),
            samples: samples.to_vec(),
            unit: Unit::G,
            accuracy_abs: 0.2,
            filtered: false,
        }],
        events: Vec::new(),
        feedback: SubjectiveFeedback {
            descriptors: descriptors.iter().map(|s| s.to_string()).collect(),
            rating: None,
            note: None,
        },
    }
}

fn diagnose(samples: &[(f64, f64)], descriptors: &[&str], grind: f64) -> adi_core::Diagnosis {
    let domain = Espresso::new();
    let mut c = cycle(samples, descriptors, grind);
    domain.derive(&mut c);
    let targets = domain
        .knowledge()
        .resolve("adi.espresso.style.espresso", None)
        .expect("the espresso style must resolve")
        .targets;
    domain.diagnose(&c, &targets)
}

/// Erratic flow with contradictory taste is channeling. The engine must not
/// prescribe a settings change, because a grind adjustment makes channeling
/// worse rather than better.
#[test]
fn channeling_is_checked_before_grind() {
    let d = diagnose(
        &[
            (0.0, 0.0),
            (5.0, 1.0),
            (8.0, 9.0),
            (10.0, 10.0),
            (14.0, 11.0),
            (16.0, 22.0),
            (20.0, 24.0),
            (21.0, 31.0),
        ],
        &["sour", "bitter"],
        24.0,
    );

    assert_eq!(d.cause, "channeling");
    assert!(
        d.target_variable.is_none(),
        "channeling must not produce an actuation; it is a preparation fault"
    );
    assert!(d.desired_delta.is_none());
}

#[test]
fn sour_and_fast_is_under_extracted() {
    let d = diagnose(
        &[
            (0.0, 0.0),
            (5.0, 2.0),
            (10.0, 8.0),
            (15.0, 15.0),
            (20.0, 24.0),
            (22.0, 30.0),
        ],
        &["sour"],
        24.0,
    );

    assert_eq!(d.cause, "under_extracted");
    assert_eq!(d.target_variable, Some(VariableId::from(vars::SHOT_TIME)));
    assert!(
        d.desired_delta.unwrap() > 0.0,
        "under-extraction wants a longer cycle"
    );
}

#[test]
fn bitter_and_slow_is_over_extracted() {
    let d = diagnose(
        &[
            (0.0, 0.0),
            (10.0, 3.0),
            (20.0, 11.0),
            (30.0, 20.0),
            (38.0, 26.0),
            (41.0, 30.0),
        ],
        &["bitter"],
        18.0,
    );

    assert_eq!(d.cause, "over_extracted");
    assert!(
        d.desired_delta.unwrap() < 0.0,
        "over-extraction wants a shorter cycle"
    );
}

#[test]
fn in_target_proposes_nothing() {
    let d = diagnose(
        // A 28 second shot, which is 28 seconds of extraction after a four
        // second pre-infusion, not a 28 second trace. ADI-P 7.3.
        &[
            (0.0, 0.0),
            (4.0, 1.0),
            (11.0, 7.0),
            (18.0, 13.0),
            (25.0, 20.0),
            (32.0, 27.0),
        ],
        &["balanced"],
        21.0,
    );

    assert_eq!(d.cause, "balanced");
    assert!(d.target_variable.is_none());
}

#[test]
fn derived_variables_are_computed() {
    let domain = Espresso::new();
    // Flow begins at 4 s and stops at 32 s, so the shot ran for 28.
    let mut c = cycle(
        &[(0.0, 0.0), (4.0, 1.0), (14.0, 8.0), (24.0, 16.0), (32.0, 27.0)],
        &["balanced"],
        21.0,
    );
    domain.derive(&mut c);

    let yielded = c.value(&VariableId::from(vars::YIELD_MASS)).unwrap();
    assert_eq!(yielded.value, 27.0);

    // 32 s of trace, 4 s of it before anything reached the cup.
    let time = c.value(&VariableId::from(vars::SHOT_TIME)).unwrap();
    assert_eq!(time.value, 28.0);

    // 27 g out of 9 g in.
    let ratio = c.value(&VariableId::from(vars::RATIO)).unwrap();
    assert!((ratio.value - 3.0).abs() < 1e-9);
}

// ---------------------------------------------------------------- planning

fn load<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/declarations")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("example file");
    serde_yaml::from_str(&text).expect("example should parse")
}

/// The skill says grind responds at -1.4 s per step. Wanting a longer cycle
/// must therefore move the setting down, which on this machine is finer.
#[test]
fn correction_moves_grind_in_the_right_direction() {
    let declaration: Declaration = load("ex900.yaml");
    let skill: Skill = load("ex900.skill.yaml");
    let domain = Espresso::new();
    let planner = Planner::new(&declaration, Some(&skill), &domain);

    let shot_time = VariableId::from(vars::SHOT_TIME);
    let candidates = planner.candidates(&shot_time);
    assert!(
        !candidates.is_empty(),
        "the machine declares tools that move cycle time"
    );

    // Highest confidence first: grind at 0.75 beats dose at 0.6 and
    // temperature at 0.5.
    assert_eq!(
        candidates[0].tool,
        SemanticId::from("adi.espresso.grind.set_setting")
    );

    let diagnosis = adi_core::Diagnosis {
        cause: "under_extracted".into(),
        target_variable: Some(shot_time),
        desired_delta: Some(4.0),
        confidence: 0.8,
        explanation: String::new(),
    };

    let correction = planner
        .plan(&diagnosis, &candidates[0], Quantity::new(24.0, Unit::Step))
        .expect("planning should succeed");

    assert!(
        correction.delta.value < 0.0,
        "a longer cycle needs a lower step number on this machine"
    );
    assert_eq!(correction.absolute.value, 22.0);
    assert_eq!(
        correction.requires_purge,
        Some(SemanticId::from("adi.espresso.grind.purge")),
        "the skill declares retention, so the engine must purge before trusting the next cycle"
    );
}

/// Without a skill the engine has no declared response, so it must not invent
/// one. See PROTOCOL.md 5.6.
#[test]
fn no_skill_means_no_candidates() {
    let declaration: Declaration = load("ex900.yaml");
    let domain = Espresso::new();
    let planner = Planner::new(&declaration, None, &domain);
    assert!(planner
        .candidates(&VariableId::from(vars::SHOT_TIME))
        .is_empty());
}

/// ADI-P 7.3 measures shot time from first drip to stop, not across the whole
/// trace. A shot spends its first seconds saturating the puck with nothing in
/// the cup, and counting that makes every shot look longer than it ran.
#[test]
fn shot_time_runs_from_first_drip_not_from_the_start() {
    use adi_core::{Cycle, ObservedEvent, SubjectiveFeedback};

    let mut cycle = Cycle {
        cycle_id: "t".into(),
        subject_id: None,
        operator_id: None,
        settings: Default::default(),
        measured: Default::default(),
        traces: vec![adi_core::Trace {
            signal: SemanticId::from("adi.espresso.signal.mass"),
            // Nothing measurable for four seconds, then a normal extraction.
            samples: vec![(0.0, 0.0), (4.0, 0.1), (12.0, 8.0), (20.0, 18.0), (32.0, 27.0)],
            unit: Unit::G,
            accuracy_abs: 0.2,
            filtered: false,
        }],
        events: Vec::new(),
        feedback: SubjectiveFeedback::default(),
    };

    // Estimated: the first sample above the scale's own accuracy is at 12 s,
    // because 0.1 g is below what it can resolve.
    adi_domain_espresso::rules::derive(&mut cycle);
    let estimated = cycle.measured[&VariableId::from("shot_time")].value;
    assert_eq!(estimated, 20.0, "32 s trace, flow seen at 12 s");

    // Reported: the appliance saw flow begin at 4 s and said so, which is
    // eight seconds it could see and the curve could not.
    cycle.measured.clear();
    cycle.events.push(ObservedEvent {
        id: SemanticId::from("adi.espresso.event.first_drip"),
        at_s: 4.0,
    });
    adi_domain_espresso::rules::derive(&mut cycle);
    let reported = cycle.measured[&VariableId::from("shot_time")].value;
    assert_eq!(reported, 28.0, "the appliance is believed over the estimate");

    assert!(
        reported > estimated,
        "the whole point: an estimate from the curve can only ever start late"
    );
}

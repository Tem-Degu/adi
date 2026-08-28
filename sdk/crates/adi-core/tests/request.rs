//! The request path: structured intent to concrete targets.
//!
//! Fictional machine and fictional domain throughout. The espresso version of
//! the same rule is the `less-sugar-never-touches-extraction` conformance case.

use adi_core::domain::{Diagnosis, Domain};
use adi_core::memory::{Memory, NoMemory, RecentCycles, Remembered};
use adi_core::request::{self, RequestError};
use adi_core::{Cycle, Planner, SubjectiveFeedback};
use adi_protocol::ids::VariableId;
use adi_protocol::intent::{Intent, IntentKind, Reference, RelativeSpec, TargetSpec};
use adi_protocol::knowledge::{Knowledge, KnowledgeBase, ResolvedTargets};
use adi_protocol::units::{Quantity, Unit};
use adi_protocol::{Declaration, Skill};
use std::collections::BTreeMap;

fn fixture<T: serde::de::DeserializeOwned>(rel: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn declaration() -> Declaration {
    fixture("examples/declarations/neutral-machine.yaml")
}

fn skill() -> Skill {
    fixture("examples/declarations/neutral-machine.skill.yaml")
}

struct TestDomain {
    variables: Vec<VariableId>,
    knowledge: Knowledge,
}

impl TestDomain {
    fn new() -> Self {
        let base: KnowledgeBase = fixture("examples/knowledge/neutral-base.yaml");
        Self {
            variables: vec![
                VariableId::from("cycle_time"),
                VariableId::from("additive_volume"),
            ],
            knowledge: Knowledge::new(base.into_layer()).unwrap(),
        }
    }
}

impl Domain for TestDomain {
    fn profile_id(&self) -> &str {
        "example/v1"
    }
    fn variables(&self) -> &[VariableId] {
        &self.variables
    }
    fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }
    fn derive(&self, _: &mut Cycle) {}
    fn diagnose(&self, _: &Cycle, _: &ResolvedTargets) -> Diagnosis {
        Diagnosis::advisory("balanced", "")
    }
    fn feedback_vocabulary(&self) -> &[&str] {
        &["good", "bad"]
    }
}

/// A machine that ran once with 8 ml of additive.
fn memory_with_last() -> RecentCycles {
    let mut m = RecentCycles::default();
    m.push(Remembered {
        cycle_id: "previous".into(),
        style: Some("adi.example.style.standard".into()),
        settings: BTreeMap::from([
            (
                VariableId::from("additive_volume"),
                Quantity::new(8.0, Unit::Ml),
            ),
            (VariableId::from("input_mass"), Quantity::new(9.0, Unit::G)),
        ]),
        feedback: SubjectiveFeedback::default(),
    });
    m
}

fn relative(of: Reference, scale: Option<f64>, offset: Option<Quantity>) -> Intent {
    Intent {
        kind: IntentKind::Adjust,
        style: None,
        material: None,
        targets: BTreeMap::from([(
            VariableId::from("additive_volume"),
            TargetSpec::Relative(RelativeSpec { of, scale, offset }),
        )]),
    }
}

// ------------------------------------------------------------- resolution

/// The headline case. The model said "half of last time"; it did not say 4, and
/// it was not asked to remember that last time was 8.
#[test]
fn half_of_last_time_resolves_against_memory() {
    let resolved = request::resolve(
        &relative(Reference::Last, Some(0.5), None),
        &memory_with_last(),
    )
    .expect("a remembered cycle exists");

    let got = resolved.targets[&VariableId::from("additive_volume")];
    assert_eq!(got.value, 4.0);
    assert_eq!(got.unit, Unit::Ml, "the unit comes from what was remembered");
}

/// Zero is a real answer: "none at all".
#[test]
fn a_scale_of_zero_is_allowed() {
    let resolved =
        request::resolve(&relative(Reference::Last, Some(0.0), None), &memory_with_last()).unwrap();
    assert_eq!(resolved.targets[&VariableId::from("additive_volume")].value, 0.0);
}

/// A negative quantity has no meaning, and would otherwise be clamped to the
/// bottom of the range as though it had been asked for.
#[test]
fn a_negative_scale_is_refused() {
    assert!(matches!(
        request::resolve(&relative(Reference::Last, Some(-1.0), None), &memory_with_last()),
        Err(RequestError::Intent(_))
    ));
}

#[test]
fn an_offset_adds_to_what_was_remembered() {
    let resolved = request::resolve(
        &relative(Reference::Last, None, Some(Quantity::new(2.0, Unit::Ml))),
        &memory_with_last(),
    )
    .unwrap();
    assert_eq!(resolved.targets[&VariableId::from("additive_volume")].value, 10.0);
}

/// Adding grams to millilitres gives a number that means nothing, and it would
/// be dispensed anyway. The same hole that was found in `adi-safety`.
#[test]
fn an_offset_in_the_wrong_unit_is_refused() {
    assert!(matches!(
        request::resolve(
            &relative(Reference::Last, None, Some(Quantity::new(2.0, Unit::G))),
            &memory_with_last()
        ),
        Err(RequestError::UnitMismatch { .. })
    ));
}

/// Neither scale nor offset means "the same as then". This is how "make it like
/// Tuesday's" resolves.
#[test]
fn neither_scale_nor_offset_means_the_same_again() {
    let resolved =
        request::resolve(&relative(Reference::Last, None, None), &memory_with_last()).unwrap();
    assert_eq!(resolved.targets[&VariableId::from("additive_volume")].value, 8.0);
}

/// The first time someone says "less than last time", there was no last time.
/// Refusing is correct; inventing a reference is not.
#[test]
fn a_relative_request_with_nothing_remembered_is_refused() {
    assert!(matches!(
        request::resolve(&relative(Reference::Last, Some(0.5), None), &NoMemory),
        Err(RequestError::NoReference { .. })
    ));
}

#[test]
fn a_named_cycle_can_be_referenced() {
    let m = memory_with_last();
    let ok = request::resolve(
        &relative(Reference::Cycle("previous".into()), Some(0.5), None),
        &m,
    );
    assert_eq!(
        ok.unwrap().targets[&VariableId::from("additive_volume")].value,
        4.0
    );

    assert!(matches!(
        request::resolve(&relative(Reference::Cycle("tuesday".into()), Some(0.5), None), &m),
        Err(RequestError::NoReference { .. })
    ));
}

/// Remembered, but that variable was never set on it.
#[test]
fn a_reference_that_did_not_set_the_variable_is_refused() {
    let intent = Intent {
        kind: IntentKind::Adjust,
        style: None,
        material: None,
        targets: BTreeMap::from([(
            VariableId::from("cycle_time"),
            TargetSpec::Relative(RelativeSpec {
                of: Reference::Last,
                scale: Some(0.5),
                offset: None,
            }),
        )]),
    };
    assert!(matches!(
        request::resolve(&intent, &memory_with_last()),
        Err(RequestError::NotSetOnReference { .. })
    ));
}

#[test]
fn an_absolute_target_needs_no_memory() {
    let intent = Intent {
        kind: IntentKind::Adjust,
        style: None,
        material: None,
        targets: BTreeMap::from([(
            VariableId::from("additive_volume"),
            TargetSpec::Absolute(Quantity::new(30.0, Unit::Ml)),
        )]),
    };
    let resolved = request::resolve(&intent, &NoMemory).unwrap();
    assert_eq!(resolved.targets[&VariableId::from("additive_volume")].value, 30.0);
}

#[test]
fn an_adjustment_naming_nothing_is_refused() {
    let intent = Intent {
        kind: IntentKind::Adjust,
        style: None,
        material: None,
        targets: BTreeMap::new(),
    };
    assert!(request::resolve(&intent, &NoMemory).is_err());
}

// -------------------------------------------------------- the preference rule

/// ADI-P 7.1 from the other side. Someone asking for less additive has asked
/// about taste. A tool that would also move the process to achieve it has
/// changed the thing they did not ask about.
///
/// The fixture offers two tools that both add the additive. One touches only
/// the additive; the other also moves container_contents. Only the first may be
/// offered.
#[test]
fn a_preference_request_may_not_use_a_tool_that_moves_the_process() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);
    let variable = VariableId::from("additive_volume");

    // Both are capable.
    let all = planner.candidates(&variable);
    assert_eq!(all.len(), 2, "the fixture must offer both, or this proves nothing");

    let allowed = request::candidates_for(&planner, domain.knowledge(), &variable).unwrap();
    assert_eq!(allowed.len(), 1);
    assert_eq!(allowed[0].tool.0, "x.example.aux.additive");
}

/// With the clean tool removed, every remaining way to serve the request would
/// disturb the process. Refusing and saying so is the right answer.
#[test]
fn a_preference_request_is_refused_when_every_tool_would_touch_the_process() {
    let d = declaration();
    let mut s = skill();
    s.responses.retain(|r| r.tool.0 != "x.example.aux.additive");
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let err = request::candidates_for(
        &planner,
        domain.knowledge(),
        &VariableId::from("additive_volume"),
    )
    .expect_err("nothing may be offered");
    assert!(matches!(
        err,
        RequestError::PreferenceWouldTouchProcess { .. }
    ));
}

/// The rule applies to preference variables only. A process request may use a
/// process tool, which is the ordinary case and must not be caught by this.
#[test]
fn a_process_request_is_not_restricted() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let allowed =
        request::candidates_for(&planner, domain.knowledge(), &VariableId::from("cycle_time"))
            .unwrap();
    assert_eq!(allowed.len(), planner.candidates(&VariableId::from("cycle_time")).len());
}

// ------------------------------------------------------------- diagnosis

/// A request and a fault are different things and the record must say which.
#[test]
fn a_request_becomes_a_diagnosis_the_planner_understands() {
    let domain = TestDomain::new();
    let k = domain.knowledge();

    let taste = request::as_diagnosis(
        &VariableId::from("additive_volume"),
        Quantity::new(4.0, Unit::Ml),
        Some(Quantity::new(8.0, Unit::Ml)),
        k,
    );
    assert_eq!(taste.cause, "preference_adjustment");
    assert_eq!(taste.desired_delta, Some(-4.0));
    assert_eq!(
        taste.confidence, 1.0,
        "a request is not an inference; the person said what they wanted"
    );

    let process = request::as_diagnosis(
        &VariableId::from("cycle_time"),
        Quantity::new(30.0, Unit::S),
        Some(Quantity::new(25.0, Unit::S)),
        k,
    );
    assert_eq!(process.cause, "requested_change");
}

// ---------------------------------------------------------------- memory

#[test]
fn memory_keeps_the_most_recent_first_and_is_bounded() {
    let mut m = RecentCycles::new(2);
    for id in ["a", "b", "c"] {
        m.push(Remembered {
            cycle_id: id.into(),
            style: None,
            settings: BTreeMap::new(),
            feedback: SubjectiveFeedback::default(),
        });
    }
    assert_eq!(m.len(), 2, "the oldest falls off");
    assert_eq!(m.last().unwrap().cycle_id, "c");
    assert!(m.get("a").is_none());
    assert_eq!(m.recent(5).len(), 2);
}

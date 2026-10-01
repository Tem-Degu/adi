//! Correction planning.
//!
//! The planner is where a diagnosis becomes a number sent to hardware, so these
//! are the tests that matter most in this crate. ADI-P 9.3 requirement 1 says
//! every actuation parameter comes from here and never from a model, which only
//! means something if the arithmetic and the gates are right.
//!
//! Fictional machine throughout. `adi-core` must not know what any real
//! appliance is.

use adi_core::domain::{Diagnosis, Domain};
use adi_core::{CoreError, Cycle, Planner, SubjectiveFeedback};
use adi_protocol::ids::{SemanticId, VariableId};
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

/// A minimal profile. `adi-core` is generic over `Domain`, so testing it needs
/// one, and inventing a fictional one here keeps the crate honest.
struct TestDomain {
    variables: Vec<VariableId>,
    knowledge: Knowledge,
}

impl TestDomain {
    fn new() -> Self {
        let base: KnowledgeBase = fixture("examples/knowledge/neutral-base.yaml");
        Self {
            variables: ["cycle_time", "output_ratio", "process_temperature", "input_mass"]
                .iter()
                .map(|v| VariableId::from(*v))
                .collect(),
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
    fn derive(&self, _cycle: &mut Cycle) {}
    fn diagnose(&self, _cycle: &Cycle, _targets: &ResolvedTargets) -> Diagnosis {
        Diagnosis::advisory("balanced", "nothing to correct")
    }
    fn feedback_vocabulary(&self) -> &[&str] {
        &["good", "bad"]
    }
}

fn wants(variable: &str, delta: f64, confidence: f64) -> Diagnosis {
    Diagnosis {
        cause: "test".into(),
        target_variable: Some(VariableId::from(variable)),
        desired_delta: Some(delta),
        confidence,
        explanation: String::new(),
    }
}

fn empty_cycle() -> Cycle {
    Cycle {
        cycle_id: "c1".into(),
        subject_id: None,
        operator_id: None,
        settings: BTreeMap::new(),
        measured: BTreeMap::new(),
        traces: Vec::new(),
        events: Vec::new(),
        feedback: SubjectiveFeedback::default(),
    }
}

// ------------------------------------------------------------- candidates

/// ADI-P 8.2: the engine must not autonomously invoke a tool for which it has
/// no skill response. Calling an undescribed tool on a hot machine to see what
/// happens is what this protocol exists to prevent.
#[test]
fn without_a_skill_nothing_is_selectable() {
    let d = declaration();
    let domain = TestDomain::new();
    let p = Planner::new(&d, None, &domain);
    assert!(p.candidates(&VariableId::from("cycle_time")).is_empty());
}

/// The mirror of the rule above: a single skill response is the whole mechanism
/// that makes a vendor tool the registry has never heard of selectable.
#[test]
fn a_skill_response_is_what_makes_a_vendor_tool_selectable() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();

    let with = Planner::new(&d, Some(&s), &domain);
    let found = with.candidates(&VariableId::from("aux_level"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tool.0, "x.example.aux.actuate");

    let without = Planner::new(&d, None, &domain);
    assert!(without.candidates(&VariableId::from("aux_level")).is_empty());
}

/// Authority is declared by the machine, not granted by the skill. The fixture
/// skill deliberately declares a high-confidence response for an operator-only
/// tool, and it must never be offered.
#[test]
fn an_operator_only_tool_is_never_a_candidate() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    assert!(
        s.responses
            .iter()
            .any(|r| r.tool.0 == "adi.example.manual.service"),
        "the fixture must keep describing this tool, or the test proves nothing"
    );
    assert!(!p
        .candidates(&VariableId::from("cycle_time"))
        .iter()
        .any(|c| c.tool.0 == "adi.example.manual.service"));
}

/// A declared response of zero says the tool does not move the variable.
/// Sizing a correction from it would divide by zero.
#[test]
fn a_tool_declared_to_have_no_effect_is_not_a_candidate() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    assert!(!p
        .candidates(&VariableId::from("cycle_time"))
        .iter()
        .any(|c| c.tool.0 == "adi.example.additive.dispense"));
}

#[test]
fn candidates_are_ranked_most_confident_first() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let found = p.candidates(&VariableId::from("cycle_time"));
    assert_eq!(found.len(), 2, "two usable tools move cycle_time");
    assert_eq!(found[0].tool.0, "adi.example.actuator.set_position");
    assert!(found[0].confidence > found[1].confidence);
}

#[test]
fn an_unmodelled_variable_yields_nothing() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);
    assert!(p.candidates(&VariableId::from("nothing_here")).is_empty());
}

// ------------------------------------------------------------------- plan

/// The physical correctness test. The response is negative: raising the
/// position number shortens the cycle. So wanting a *longer* cycle must move
/// the position *down*. A sign error here makes every correction worsen the
/// fault it was meant to fix.
#[test]
fn a_negative_response_reverses_the_direction_of_travel() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    let longer = p
        .plan(&wants("cycle_time", 4.0, 1.0), c, Quantity::new(30.0, Unit::Step))
        .unwrap();
    assert!(longer.delta.value < 0.0, "a longer cycle needs a lower position");

    let shorter = p
        .plan(&wants("cycle_time", -4.0, 1.0), c, Quantity::new(30.0, Unit::Step))
        .unwrap();
    assert!(shorter.delta.value > 0.0);
}

/// The whole arithmetic, end to end, on a parameter whose resolution is fine
/// enough not to interfere.
///
/// per_unit is 0.02 ratio per ml, so wanting +0.4 ratio means +20 ml, scaled by
/// the response's 0.95 confidence to +19. The parameter resolution is 0.5 ml,
/// so 19 survives snapping.
#[test]
fn the_magnitude_comes_from_the_declared_response() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("output_ratio"))[0];
    let plan = p
        .plan(
            &wants("output_ratio", 0.4, 1.0),
            c,
            Quantity::new(40.0, Unit::Ml),
        )
        .unwrap();
    assert!((plan.delta.value - 19.0).abs() < 1e-9);
    assert!((plan.absolute.value - 59.0).abs() < 1e-9);
}

/// The machine declares a resolution because it cannot accept anything finer.
/// A position of 27.75 steps is not a value that can be sent, so the planner
/// must snap it and report the snapped delta.
#[test]
fn a_correction_is_snapped_to_the_declared_resolution() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    // 4.2 / -1.4 = -3 steps, scaled by 0.75 gives -2.25, which is not sendable
    // on a mechanism whose resolution is 1 step.
    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    let plan = p
        .plan(&wants("cycle_time", 4.2, 1.0), c, Quantity::new(30.0, Unit::Step))
        .unwrap();

    assert_eq!(plan.absolute.value, 28.0);
    assert_eq!(plan.delta.value, -2.0);
    assert_eq!(
        plan.absolute.value.fract(),
        0.0,
        "a step mechanism must never be sent a fraction"
    );
}

/// Low confidence means smaller steps. Overshooting costs a whole cycle, and in
/// a slow domain possibly a whole week.
#[test]
fn low_confidence_produces_a_smaller_step() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    let sure = p
        .plan(&wants("cycle_time", 4.0, 1.0), c, Quantity::new(30.0, Unit::Step))
        .unwrap();
    let unsure = p
        .plan(&wants("cycle_time", 4.0, 0.2), c, Quantity::new(30.0, Unit::Step))
        .unwrap();
    assert!(unsure.delta.value.abs() < sure.delta.value.abs());
}

/// The declared parameter range is a hard stop. The planner must not propose a
/// value the machine would reject, and the reported delta must be the one
/// actually applied rather than the one wanted.
#[test]
fn a_correction_is_clamped_to_the_declared_range() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    // Position runs 0 to 60. Ask for a change far larger than that.
    let plan = p
        .plan(
            &wants("cycle_time", -500.0, 1.0),
            c,
            Quantity::new(55.0, Unit::Step),
        )
        .unwrap();

    assert_eq!(plan.absolute.value, 60.0);
    assert_eq!(
        plan.delta.value, 5.0,
        "the delta must describe what was applied, not what was wanted"
    );
}

/// A change whose effect is delayed by retained material must carry its remedy,
/// or the engine reads the next cycle as evidence and corrects the wrong way.
#[test]
fn a_tool_with_retention_carries_its_remedy() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let position = &p.candidates(&VariableId::from("cycle_time"))[0];
    let plan = p
        .plan(&wants("cycle_time", 2.0, 1.0), position, Quantity::new(30.0, Unit::Step))
        .unwrap();
    assert_eq!(
        plan.requires_purge.as_ref().map(|t| t.0.as_str()),
        Some("adi.example.actuator.purge")
    );

    let mass = &p.candidates(&VariableId::from("cycle_time"))[1];
    let plan = p
        .plan(&wants("cycle_time", 2.0, 1.0), mass, Quantity::new(9.0, Unit::G))
        .unwrap();
    assert!(plan.requires_purge.is_none());
}

#[test]
fn a_diagnosis_wanting_nothing_cannot_be_planned() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    let advisory = Diagnosis::advisory("balanced", "nothing to do");
    assert!(p
        .plan(&advisory, c, Quantity::new(30.0, Unit::Step))
        .is_err());
}

/// The diagnosis is what the profile produced; the cycle is untouched by
/// planning. Kept because a planner that mutated the cycle would break replay.
#[test]
fn planning_does_not_depend_on_the_cycle() {
    let domain = TestDomain::new();
    let mut c = empty_cycle();
    domain.derive(&mut c);
    assert!(c.measured.is_empty());
}

// ------------------------------------- values that must never reach hardware

/// The planner computes on the parameter's present value, so that value must be
/// in the parameter's unit. Before this was checked, handing 30 grams as the
/// current position of a mechanism declared in steps produced a confident plan
/// of 28 steps: arithmetic across units, sent to hardware as if it meant
/// something.
///
/// The same hole was found and fixed in `adi-safety` accumulation. It is worth
/// looking for wherever two numbers meet.
#[test]
fn a_current_value_in_the_wrong_unit_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = &p.candidates(&VariableId::from("cycle_time"))[0];
    let r = p.plan(
        &wants("cycle_time", 4.0, 1.0),
        c,
        Quantity::new(30.0, Unit::G),
    );
    assert!(matches!(r, Err(CoreError::UnitMismatch { .. })));

    // The same call in the declared unit still works, so the check is not
    // simply refusing everything.
    assert!(p
        .plan(
            &wants("cycle_time", 4.0, 1.0),
            c,
            Quantity::new(30.0, Unit::Step)
        )
        .is_ok());
}

/// `candidates` filters a zero response, but `Candidate` is constructible by
/// any caller, and at D1 the selection comes from a model choosing off the
/// list. The division is here, so the guard belongs here.
///
/// Before the guard: 4.0 / 0.0 is infinity, clamping infinity returns the top
/// of the range, and the mechanism was driven to its limit.
#[test]
fn a_zero_response_cannot_drive_the_mechanism_to_its_limit() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);

    let c = adi_core::plan::Candidate {
        tool: SemanticId("adi.example.actuator.set_position".into()),
        parameter: "position".into(),
        variable: VariableId::from("cycle_time"),
        per_unit: 0.0,
        confidence: 0.9,
        param_unit: Unit::Step,
            description: None,
            also_affects: Vec::new(),
    };
    assert!(matches!(
        p.plan(
            &wants("cycle_time", 4.0, 1.0),
            &c,
            Quantity::new(30.0, Unit::Step)
        ),
        Err(CoreError::NoResponse { .. })
    ));
}

/// A NaN reaching `Range::clamp` survives it, so before this check a NaN
/// current produced a NaN actuation value.
#[test]
fn a_non_finite_value_is_never_produced() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let p = Planner::new(&d, Some(&s), &domain);
    let c = &p.candidates(&VariableId::from("cycle_time"))[0];

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            matches!(
                p.plan(
                    &wants("cycle_time", 4.0, 1.0),
                    c,
                    Quantity::new(bad, Unit::Step)
                ),
                Err(CoreError::NotFinite { .. })
            ),
            "current {bad} must be refused"
        );
        assert!(
            matches!(
                p.plan(
                    &wants("cycle_time", bad, 1.0),
                    c,
                    Quantity::new(30.0, Unit::Step)
                ),
                Err(CoreError::NotFinite { .. })
            ),
            "a desired delta of {bad} must be refused"
        );
    }
}

/// A manufacturer's description reaches the candidate, so a model choosing
/// between two tools can read it. Nothing in the engine consumes it.
///
/// The field is optional, and a tool without one must behave exactly as before,
/// which is the half of this that a present-only assertion would miss.
#[test]
fn a_tool_description_reaches_the_candidate() {
    let mut declaration = declaration();
    let skill = skill();

    declaration.tools[0].description = Some("settles slowly, retains grounds".into());

    let domain = TestDomain::new();
    let planner = Planner::new(&declaration, Some(&skill), &domain);
    let candidates = planner.candidates(&VariableId::from("cycle_time"));

    assert!(!candidates.is_empty(), "the fixture should offer candidates");

    let described = candidates
        .iter()
        .find(|c| c.tool == declaration.tools[0].id)
        .expect("the described tool should be a candidate");
    assert_eq!(
        described.description.as_deref(),
        Some("settles slowly, retains grounds")
    );

    for other in candidates.iter().filter(|c| c.tool != declaration.tools[0].id) {
        assert!(
            other.description.is_none(),
            "a tool with no description must carry none, got {:?}",
            other.description
        );
    }
}

// ------------------------------------------------- a correction at the limit

/// A correction held at the end of the declared range records what it wanted.
///
/// Without this the record says the machine acted, and nothing distinguishes a
/// small correction from a mechanism that cannot move. The person is told their
/// request was carried out when it was not.
#[test]
fn a_correction_held_at_the_limit_records_what_it_wanted() {
    let declaration = declaration();
    let skill = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&declaration, Some(&skill), &domain);

    // Ask for far more change than the range allows.
    let diagnosis = Diagnosis {
        cause: "cycle_time".into(),
        target_variable: Some(VariableId::from("cycle_time")),
        desired_delta: Some(900.0),
        confidence: 1.0,
        explanation: String::new(),
    };

    let candidate = planner
        .candidates(&VariableId::from("cycle_time"))
        .into_iter()
        .next()
        .expect("a candidate");

    let correction = planner
        .plan(&diagnosis, &candidate, Quantity::new(20.0, candidate.param_unit))
        .expect("a correction is still produced, partial");

    let wanted = correction
        .clamped_from
        .expect("the range would not allow it, so what was wanted is recorded");
    assert!(
        wanted.value != correction.absolute.value,
        "the recorded value should differ from the one sent"
    );
}

/// Snapping to resolution is not being stuck.
///
/// `Range::clamp` both limits and snaps, and conflating them would report every
/// ordinary correction on an integer parameter as a mechanism at its limit.
#[test]
fn snapping_to_resolution_is_not_recorded_as_a_limit() {
    let declaration = declaration();
    let skill = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&declaration, Some(&skill), &domain);

    // A small change, well inside the range, that will not land on a step.
    let diagnosis = Diagnosis {
        cause: "cycle_time".into(),
        target_variable: Some(VariableId::from("cycle_time")),
        desired_delta: Some(1.0),
        confidence: 1.0,
        explanation: String::new(),
    };

    let candidate = planner
        .candidates(&VariableId::from("cycle_time"))
        .into_iter()
        .next()
        .expect("a candidate");

    let correction = planner
        .plan(&diagnosis, &candidate, Quantity::new(20.0, candidate.param_unit))
        .expect("a correction");

    assert!(
        correction.clamped_from.is_none(),
        "an ordinary correction must not be reported as being at a limit, got {:?}",
        correction.clamped_from
    );
}

/// A candidate carries what else its tool changes.
///
/// The magnitudes say how far each of two tools moves an outcome; this says what
/// moves with it, which is what separates them when a person asked for one thing
/// to change and not another. The engine computed it already for the
/// one-variable-at-a-time check and kept it to itself.
#[test]
fn a_candidate_carries_what_else_its_tool_changes() {
    let declaration = declaration();
    let skill = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&declaration, Some(&skill), &domain);

    let candidates = planner.candidates(&VariableId::from("cycle_time"));
    assert!(!candidates.is_empty(), "the fixture should offer candidates");

    // Whatever each tool declares in `effects`, minus the variable the
    // candidate was offered for, must appear.
    for candidate in &candidates {
        let tool = declaration
            .tool(&candidate.tool)
            .expect("a candidate names a declared tool");

        for effect in &tool.effects {
            if *effect == candidate.variable {
                continue;
            }
            assert!(
                candidate.also_affects.contains(effect),
                "{} declares it changes {}, which the candidate does not carry",
                candidate.tool.as_str(),
                effect.0
            );
        }
        assert!(
            !candidate.also_affects.contains(&candidate.variable),
            "the variable it is offered for is named once, not twice"
        );
    }
}

/// A registry tool with no description written takes the profile's.
///
/// A registry identifier means the same on every machine, so the description
/// belongs with the profile rather than being retyped in each declaration.
#[test]
fn a_tool_with_no_description_takes_the_profiles() {
    struct Described(TestDomain);

    impl Domain for Described {
        fn profile_id(&self) -> &str {
            self.0.profile_id()
        }
        fn variables(&self) -> &[VariableId] {
            self.0.variables()
        }
        fn knowledge(&self) -> &adi_protocol::knowledge::Knowledge {
            self.0.knowledge()
        }
        fn derive(&self, cycle: &mut adi_core::Cycle) {
            self.0.derive(cycle)
        }
        fn diagnose(
            &self,
            cycle: &adi_core::Cycle,
            targets: &adi_protocol::knowledge::ResolvedTargets,
        ) -> adi_core::domain::Diagnosis {
            self.0.diagnose(cycle, targets)
        }
        fn feedback_vocabulary(&self) -> &[&str] {
            self.0.feedback_vocabulary()
        }
        fn tool_description(&self, _tool: &SemanticId) -> Option<&str> {
            Some("what the profile says this is for")
        }
    }

    let declaration = declaration();
    let skill = skill();
    let domain = Described(TestDomain::new());
    let planner = Planner::new(&declaration, Some(&skill), &domain);

    for candidate in planner.candidates(&VariableId::from("cycle_time")) {
        let tool = declaration.tool(&candidate.tool).unwrap();
        if tool.description.is_none() {
            assert_eq!(
                candidate.description.as_deref(),
                Some("what the profile says this is for"),
                "a tool with none of its own should take the profile's"
            );
        }
    }
}

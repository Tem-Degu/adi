//! Autonomy level behaviour.
//!
//! The claims worth testing: a model can never select a tool the machine did
//! not declare, a model-supplied magnitude is still clamped and still carries a
//! unit, and D2 cannot be switched on against a machine that lacks the stateful
//! constraints which make a model-chosen magnitude safe.
//!
//! The fixture is a fictional machine in a fictional domain, so this crate's
//! neutrality is demonstrated rather than asserted.

use adi_core::domain::Diagnosis;
use adi_core::plan::{Candidate, Planner};
use adi_core::record::SelectedBy;
use adi_decision::{
    ActionChooser, ActionSelector, AutonomyLevel, DecisionError, EngineSelector,
    ModelActionSelector, ModelToolSelector, ProposedStep, SelectionContext, ToolChooser,
};
use adi_protocol::ids::VariableId;
use adi_protocol::units::{Quantity, Unit};
use adi_protocol::{Declaration, SemanticId, Skill};
use std::collections::BTreeMap;

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/declarations")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("fixture");
    serde_yaml::from_str(&text).expect("fixture should parse")
}

fn declaration() -> Declaration {
    fixture("neutral-machine.yaml")
}

fn skill() -> Skill {
    fixture("neutral-machine.skill.yaml")
}

fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            tool: SemanticId::from("adi.example.actuator.set_position"),
            parameter: "position".into(),
            variable: VariableId::from("cycle_time"),
            per_unit: -1.4,
            confidence: 0.75,
            param_unit: Unit::Step,
            description: None,
            also_affects: Vec::new(),
        },
        Candidate {
            tool: SemanticId::from("adi.example.input.by_mass"),
            parameter: "mass".into(),
            variable: VariableId::from("cycle_time"),
            per_unit: 1.1,
            confidence: 0.6,
            param_unit: Unit::G,
            description: None,
            also_affects: Vec::new(),
        },
    ]
}

/// One goal, which is what a correction produces.
fn goals() -> Vec<Diagnosis> {
    vec![diagnosis()]
}

fn wants(variable: &str, delta: f64) -> Diagnosis {
    Diagnosis {
        cause: "requested_change".into(),
        target_variable: Some(VariableId::from(variable)),
        desired_delta: Some(delta),
        confidence: 1.0,
        explanation: String::new(),
    }
}

fn diagnosis() -> Diagnosis {
    Diagnosis {
        cause: "below_target".into(),
        target_variable: Some(VariableId::from("cycle_time")),
        desired_delta: Some(4.0),
        confidence: 0.8,
        explanation: String::new(),
    }
}

fn ctx() -> SelectionContext<'static> {
    SelectionContext::default()
}

struct FixedChoice(usize);

impl ToolChooser for FixedChoice {
    fn choose(
        &self,
        goals: &[adi_decision::Goal],
        _: &[Candidate],
        _: &SelectionContext<'_>,
    ) -> Result<Vec<usize>, String> {
        Ok(vec![self.0; goals.len()])
    }
}

/// Proposes a plan verbatim, so a test can hand the engine anything a model
/// might and see what survives.
struct FixedPlan(Vec<ProposedStep>);

impl ActionChooser for FixedPlan {
    fn choose(
        &self,
        _: &[adi_decision::Goal],
        _: &[Candidate],
        _: &SelectionContext<'_>,
    ) -> Result<Vec<ProposedStep>, String> {
        Ok(self.0.clone())
    }
}

fn one(candidate: usize, value: f64) -> FixedPlan {
    FixedPlan(vec![ProposedStep { candidate, value }])
}

/// A fictional profile. `Planner` needs a `Domain`, and this crate may not know
/// what any real appliance is.
struct TestDomain {
    variables: Vec<VariableId>,
    knowledge: adi_protocol::Knowledge,
}

impl TestDomain {
    fn new() -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/knowledge/neutral-base.yaml");
        let text = std::fs::read_to_string(&path).expect("neutral base");
        let base: adi_protocol::KnowledgeBase = serde_yaml::from_str(&text).expect("parses");
        Self {
            variables: vec![VariableId::from("cycle_time")],
            knowledge: adi_protocol::Knowledge::new(base.into_layer()).unwrap(),
        }
    }
}

impl adi_core::domain::Domain for TestDomain {
    fn profile_id(&self) -> &str {
        "example/v1"
    }
    fn variables(&self) -> &[VariableId] {
        &self.variables
    }
    fn knowledge(&self) -> &adi_protocol::Knowledge {
        &self.knowledge
    }
    fn derive(&self, _: &mut adi_core::Cycle) {}
    fn diagnose(&self, _: &adi_core::Cycle, _: &adi_protocol::ResolvedTargets) -> Diagnosis {
        Diagnosis::advisory("balanced", "nothing to correct")
    }
    fn feedback_vocabulary(&self) -> &[&str] {
        &["good", "bad"]
    }
}

// ------------------------------------------------------------------ levels

/// D2 is the default. A machine that can only do what someone anticipated is a
/// program with a microphone attached, and the safety of a model-chosen value
/// does not rest on trusting the model.
#[test]
fn the_default_level_is_d2() {
    assert_eq!(AutonomyLevel::default(), AutonomyLevel::D2);
}

/// The audit trail has to say who chose. D0 is replayable; the others are not,
/// and the record must not imply otherwise.
#[test]
fn the_record_says_who_chose() {
    assert_eq!(AutonomyLevel::D0.selected_by(), SelectedBy::Engine);
    assert_eq!(AutonomyLevel::D1.selected_by(), SelectedBy::Model);
    assert_eq!(AutonomyLevel::D2.selected_by(), SelectedBy::Model);
}

#[test]
fn d0_takes_the_most_confident_candidate() {
    let selection = EngineSelector
        .select(&goals(), &candidates(), &ctx())
        .expect("a candidate exists");
    assert_eq!(selection.steps.len(), 1);
    assert_eq!(
        selection.steps[0].tool,
        SemanticId::from("adi.example.actuator.set_position")
    );
    assert!(
        selection.steps[0].value.is_none(),
        "D0 never carries a model-supplied value"
    );
    assert_eq!(EngineSelector.level(), AutonomyLevel::D0);
}

#[test]
fn d1_lets_the_model_pick_a_different_tool() {
    let selector = ModelToolSelector {
        chooser: FixedChoice(1),
    };
    let selection = selector
        .select(&goals(), &candidates(), &ctx())
        .expect("index 1 is valid");
    assert_eq!(
        selection.steps[0].tool,
        SemanticId::from("adi.example.input.by_mass"),
        "the model chose the lower-confidence tool, which is its prerogative"
    );
    assert!(
        selection.steps[0].value.is_none(),
        "D1 hands the model the tool, never the number"
    );
}

/// The model answers with an index into a list the engine built, so an
/// out-of-range answer is caught rather than becoming an undeclared tool call.
#[test]
fn a_model_cannot_select_a_tool_that_was_not_offered() {
    let d1 = ModelToolSelector {
        chooser: FixedChoice(99),
    };
    assert!(matches!(
        d1.select(&goals(), &candidates(), &ctx()),
        Err(DecisionError::UndeclaredSelection(_))
    ));

    let d2 = ModelActionSelector::new(one(99, 21.0), &declaration()).unwrap();
    assert!(matches!(
        d2.select(&goals(), &candidates(), &ctx()),
        Err(DecisionError::UndeclaredSelection(_))
    ));
}

#[test]
fn a_model_gets_no_candidates_rather_than_improvising() {
    let selector = ModelToolSelector {
        chooser: FixedChoice(0),
    };
    assert!(matches!(
        selector.select(&goals(), &[], &ctx()),
        Err(DecisionError::NoCandidate(_))
    ));
}

#[test]
fn d2_refuses_a_machine_without_stateful_constraints() {
    let mut bare = declaration();
    bare.envelope.invariants.clear();
    bare.envelope.rate_limits.clear();

    assert!(matches!(
        ModelActionSelector::new(one(0, 21.0), &bare),
        Err(DecisionError::StatefulEnvelopeRequired)
    ));
}

/// `selector_for` refuses rather than falling back to D0, because a record
/// saying D0 when D2 was asked for is a lie in the audit trail.
#[test]
fn selector_for_refuses_the_model_levels_rather_than_downgrading() {
    let d = declaration();
    assert!(adi_decision::selector_for(AutonomyLevel::D0, &d).is_ok());
    assert!(matches!(
        adi_decision::selector_for(AutonomyLevel::D1, &d),
        Err(DecisionError::NoModelBackend("D1"))
    ));
    assert!(matches!(
        adi_decision::selector_for(AutonomyLevel::D2, &d),
        Err(DecisionError::NoModelBackend("D2"))
    ));

    // On a machine with no stateful envelope, D2 reports the harder failure
    // first: the envelope, not the missing backend.
    let mut bare = declaration();
    bare.envelope.invariants.clear();
    bare.envelope.rate_limits.clear();
    assert!(matches!(
        adi_decision::selector_for(AutonomyLevel::D2, &bare),
        Err(DecisionError::StatefulEnvelopeRequired)
    ));
}

// -------------------------------------------------------- D2 magnitudes

/// A magnitude from a model arrives tagged with the parameter's declared unit,
/// not bare. An untagged number was previously passed along as `Unit::None`,
/// which meant nothing and would now be refused by the planner.
#[test]
fn a_model_supplied_value_carries_the_parameters_unit() {
    let selector = ModelActionSelector::new(one(0, 21.0), &declaration()).unwrap();
    let selection = selector
        .select(&goals(), &candidates(), &ctx())
        .expect("a valid plan");

    let value = selection.steps[0].value.expect("D2 supplies the magnitude");
    assert_eq!(value.value, 21.0);
    assert_eq!(value.unit, Unit::Step, "not Unit::None");
}

#[test]
fn d2_refuses_an_empty_plan() {
    let selector = ModelActionSelector::new(FixedPlan(vec![]), &declaration()).unwrap();
    assert!(matches!(
        selector.select(&goals(), &candidates(), &ctx()),
        Err(DecisionError::EmptyPlan)
    ));
}

fn present() -> BTreeMap<String, Quantity> {
    BTreeMap::from([
        ("position".to_string(), Quantity::new(30.0, Unit::Step)),
        ("mass".to_string(), Quantity::new(9.0, Unit::G)),
        ("amount".to_string(), Quantity::new(8.0, Unit::Ml)),
        ("volume".to_string(), Quantity::new(8.0, Unit::Ml)),
    ])
}

/// The whole point of letting a model choose the number: it is still clamped.
/// Position runs 0 to 60, so a model asking for 500 gets 60, and the record
/// shows what was actually applied.
#[test]
fn a_model_supplied_value_is_still_clamped() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);
    let selector = ModelActionSelector::new(one(0, 500.0), &d).unwrap();

    let corrections = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect("the plan should be accepted after clamping");

    assert_eq!(corrections.len(), 1);
    assert_eq!(corrections[0].absolute.value, 60.0);
    assert_eq!(corrections[0].absolute.unit, Unit::Step);
    assert_eq!(corrections[0].delta.value, 30.0);
}

/// A step mechanism cannot accept a fraction, whoever chose the number.
#[test]
fn a_model_supplied_value_is_snapped_to_the_declared_resolution() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);
    let selector = ModelActionSelector::new(one(0, 27.4), &d).unwrap();

    let corrections =
        adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present()).unwrap();
    assert_eq!(corrections[0].absolute.value, 27.0);
}

/// ADI-P 9.5. Two tools reaching the same variable in one iteration means the
/// next cycle cannot attribute the change to either, so the plan is refused
/// whole rather than half-performed.
#[test]
fn a_plan_moving_one_variable_twice_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    // Both candidates move cycle_time.
    let selector = ModelActionSelector::new(
        FixedPlan(vec![
            ProposedStep {
                candidate: 0,
                value: 25.0,
            },
            ProposedStep {
                candidate: 1,
                value: 10.0,
            },
        ]),
        &d,
    )
    .unwrap();

    let err = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect_err("two steps on one variable must be refused");
    assert!(matches!(err, DecisionError::ConflictingSteps { .. }));
}

/// A chooser that proposes a conflicting plan and then a valid one succeeds.
///
/// Before the retry existed this failed the whole cycle. The model had never
/// been shown ADI-P 9.5 and got no chance to correct a mistake that a single
/// sentence of feedback resolves.
#[test]
fn a_recoverable_mistake_gets_one_more_attempt() {
    use std::cell::Cell;
    use std::rc::Rc;

    /// Conflicts on the first call and answers properly on the second.
    ///
    /// The counters are shared with the test rather than held inside, so the
    /// test can assert a retry happened. Asserting only the result would pass
    /// if the first attempt had somehow succeeded, proving nothing about the
    /// retry this test exists for.
    struct ConflictsOnce {
        calls: Rc<Cell<usize>>,
        saw_rejection: Rc<Cell<bool>>,
    }

    impl ActionChooser for ConflictsOnce {
        fn choose(
            &self,
            _: &[adi_decision::Goal],
            _: &[Candidate],
            context: &SelectionContext<'_>,
        ) -> Result<Vec<ProposedStep>, String> {
            self.calls.set(self.calls.get() + 1);

            if self.calls.get() == 1 {
                assert!(
                    context.rejected.is_none(),
                    "a first attempt carries no rejection"
                );
                // Two steps, both moving cycle_time.
                return Ok(vec![
                    ProposedStep {
                        candidate: 0,
                        value: 25.0,
                    },
                    ProposedStep {
                        candidate: 1,
                        value: 10.0,
                    },
                ]);
            }

            self.saw_rejection.set(context.rejected.is_some());
            Ok(vec![ProposedStep {
                candidate: 0,
                value: 25.0,
            }])
        }
    }

    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let calls = Rc::new(Cell::new(0));
    let saw_rejection = Rc::new(Cell::new(false));
    let selector = ModelActionSelector::new(
        ConflictsOnce {
            calls: Rc::clone(&calls),
            saw_rejection: Rc::clone(&saw_rejection),
        },
        &d,
    )
    .unwrap();

    let out = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect("the second attempt should succeed");

    assert_eq!(calls.get(), 2, "the chooser should have been asked twice");
    assert!(
        saw_rejection.get(),
        "the second attempt must be told why the first was refused"
    );
    assert_eq!(out.len(), 1, "one correction from the valid second plan");
}

/// The retry is bounded at one. A backend that is broken rather than mistaken
/// must surface as a failure, not loop.
#[test]
fn a_chooser_that_always_conflicts_fails_after_exactly_two_attempts() {
    use std::cell::Cell;
    use std::rc::Rc;

    struct AlwaysConflicts(Rc<Cell<usize>>);

    impl ActionChooser for AlwaysConflicts {
        fn choose(
            &self,
            _: &[adi_decision::Goal],
            _: &[Candidate],
            _: &SelectionContext<'_>,
        ) -> Result<Vec<ProposedStep>, String> {
            self.0.set(self.0.get() + 1);
            Ok(vec![
                ProposedStep {
                    candidate: 0,
                    value: 25.0,
                },
                ProposedStep {
                    candidate: 1,
                    value: 10.0,
                },
            ])
        }
    }

    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let calls = Rc::new(Cell::new(0));
    let selector = ModelActionSelector::new(AlwaysConflicts(Rc::clone(&calls)), &d).unwrap();
    let err = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect_err("a plan that always conflicts must fail");

    assert_eq!(calls.get(), 2, "exactly two attempts, never a third");
    assert!(
        matches!(err, DecisionError::ConflictingSteps { .. }),
        "{err:?}"
    );
}

/// D0 is deterministic, so a retry would ask the same question and receive the
/// same answer. Retrying it would delay a failure the caller needs to see.
#[test]
fn a_deterministic_selector_is_not_retried() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    // Two goals on one variable is refused before any selector is consulted,
    // which is the behaviour being preserved: D0 must fail on its own terms.
    let err = adi_decision::decide(
        &planner,
        &EngineSelector,
        &[goals()[0].clone(), goals()[0].clone()],
        &ctx(),
        &present(),
    )
    .expect_err("duplicate goals must be refused");

    assert!(matches!(err, DecisionError::DuplicateGoal(_)), "{err:?}");
}

/// A plan that is wrong in its last step must perform none of its first.
#[test]
fn a_plan_with_a_bad_step_performs_nothing() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let selector = ModelActionSelector::new(
        FixedPlan(vec![
            ProposedStep {
                candidate: 0,
                value: 25.0,
            },
            ProposedStep {
                candidate: 7,
                value: 10.0,
            },
        ]),
        &d,
    )
    .unwrap();

    assert!(adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present()).is_err());
}

/// Without a present value for the parameter there is nothing to correct from,
/// and assuming zero would send a mechanism to one end of its travel.
#[test]
fn a_step_with_no_present_value_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);
    let selector = ModelActionSelector::new(one(0, 25.0), &d).unwrap();

    assert!(adi_decision::decide(&planner, &selector, &goals(), &ctx(), &BTreeMap::new()).is_err());
}

// --------------------------------------------- a correction that corrects

/// The defect this group exists for.
///
/// A value equal to the setting already in force moves nothing. Before this,
/// the engine clamped it, found it in range, sent it, recorded a correction
/// and explained the fault it had not corrected, so the machine reported
/// having acted on every cycle while the fault persisted. Qwen3 4B proposed
/// exactly this on every cycle of a real session, and the session reported
/// success each time.
#[test]
fn a_value_equal_to_the_present_setting_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    // `position` is at 30 in `present()`.
    let selector = ModelActionSelector::new(one(0, 30.0), &d).unwrap();
    let err = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect_err("a correction that changes nothing is not a correction");

    let text = err.to_string();
    assert!(
        text.contains("already in") && text.contains("change nothing"),
        "the refusal should say the value is the one in force: {text}"
    );
}

/// The same thing through the resolution. A proposal that is not the present
/// setting but rounds onto it moves nothing either, and the rounding happens
/// before this is judged.
#[test]
fn a_value_that_rounds_onto_the_present_setting_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    // `position` is declared in whole steps, so 30.4 is sent as 30.
    let selector = ModelActionSelector::new(one(0, 30.4), &d).unwrap();
    assert!(
        adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present()).is_err(),
        "30.4 snaps to the present 30 and moves nothing"
    );
}

/// A mechanism at the end of its travel is a different situation, and the
/// refusal says so, because the correction is to choose another candidate
/// rather than to propose a different number for this one.
#[test]
fn a_mechanism_already_at_its_limit_says_so() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    // `position` is declared 0 to 60. Hold it at the top and ask for more.
    let mut at_limit = present();
    at_limit.insert("position".to_string(), Quantity::new(60.0, Unit::Step));

    let selector = ModelActionSelector::new(one(0, 70.0), &d).unwrap();
    let err = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &at_limit)
        .expect_err("a mechanism with nothing left to give cannot serve the goal");

    let text = err.to_string();
    assert!(
        text.contains("end of its declared range"),
        "the refusal should name the situation: {text}"
    );
    assert!(
        text.contains("different candidate"),
        "and say what would help: {text}"
    );
}

/// The refusal must not catch a real correction, which is the whole point of
/// comparing against the present setting rather than against a tolerance.
#[test]
fn a_correction_that_moves_the_parameter_is_untouched() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let selector = ModelActionSelector::new(one(0, 25.0), &d).unwrap();
    let corrections = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect("moving position from 30 to 25 is a correction");

    assert_eq!(corrections[0].absolute.value, 25.0);
    assert_eq!(corrections[0].delta.value, -5.0);
}

/// ADI-P 9.4.6: a refusal the model could correct earns one further attempt,
/// carrying the reason. A model told that its value was the setting already in
/// force can propose one that moves, and the cycle then completes.
#[test]
fn a_model_told_its_value_changed_nothing_can_correct_it() {
    use std::cell::Cell;
    use std::rc::Rc;

    struct NoChangeOnce {
        calls: Rc<Cell<usize>>,
        saw_rejection: Rc<Cell<bool>>,
    }

    impl ActionChooser for NoChangeOnce {
        fn choose(
            &self,
            _: &[adi_decision::Goal],
            _: &[Candidate],
            context: &SelectionContext<'_>,
        ) -> Result<Vec<ProposedStep>, String> {
            self.calls.set(self.calls.get() + 1);

            if self.calls.get() == 1 {
                // The present setting, which is what the small model proposed.
                return Ok(vec![ProposedStep {
                    candidate: 0,
                    value: 30.0,
                }]);
            }

            self.saw_rejection.set(context.rejected.is_some());
            Ok(vec![ProposedStep {
                candidate: 0,
                value: 23.0,
            }])
        }
    }

    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let calls = Rc::new(Cell::new(0));
    let saw_rejection = Rc::new(Cell::new(false));
    let selector = ModelActionSelector::new(
        NoChangeOnce {
            calls: Rc::clone(&calls),
            saw_rejection: Rc::clone(&saw_rejection),
        },
        &d,
    )
    .unwrap();

    let corrections = adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present())
        .expect("the second attempt moves the parameter");

    assert_eq!(calls.get(), 2, "the refusal must be retried once");
    assert!(
        saw_rejection.get(),
        "the second attempt must be told why the first was refused"
    );
    assert_eq!(corrections[0].absolute.value, 23.0);
}

// ------------------------------------------------------------ the model seam

/// The placeholder. A build with no model refuses to parse speech and says so,
/// rather than silently ignoring it or inventing an intent.
#[test]
fn without_a_model_speech_is_refused_not_guessed() {
    use adi_decision::{Interpreter, NoInterpreter};
    assert!(matches!(
        NoInterpreter.interpret("half as much as last time", &ctx()),
        Err(DecisionError::NoModelBackend("intent"))
    ));
}

/// The reasoning is the model's job, so it has to be given what reasoning
/// needs. It sees the same remembered cycles the engine resolves against.
/// Without this it would be asked to interpret "last time" without being told
/// what last time was.
#[test]
fn the_model_sees_what_the_machine_remembers() {
    use adi_core::memory::{RecentCycles, Remembered};
    use adi_protocol::ids::VariableId;

    let mut m = RecentCycles::default();
    m.push(Remembered {
        cycle_id: "previous".into(),
        style: None,
        settings: BTreeMap::from([(
            VariableId::from("additive_volume"),
            Quantity::new(8.0, Unit::Ml),
        )]),
        feedback: adi_core::SubjectiveFeedback::default(),
    });

    let context = SelectionContext::default().with_memory(&m);
    let seen = context.memory.expect("the model is given memory");
    assert_eq!(
        seen.last()
            .and_then(|r| r.setting(&VariableId::from("additive_volume")))
            .map(|q| q.value),
        Some(8.0)
    );

    // A machine that has run nothing shows nothing, rather than pretending.
    assert!(SelectionContext::default().memory.is_none());
}

/// Asking is an outcome, not a failure. The alternative to asking is guessing,
/// and on this path a guess becomes a physical action.
#[test]
fn a_model_may_ask_rather_than_guess() {
    use adi_decision::{Interpreter, Understood};

    struct Unsure;
    impl Interpreter for Unsure {
        fn interpret(
            &self,
            _: &str,
            _: &SelectionContext<'_>,
        ) -> Result<Understood, DecisionError> {
            Ok(Understood::Clarify {
                question: "Which one, the one this morning or yesterday's?".into(),
                because: "two remembered cycles match the description".into(),
            })
        }
    }

    let asked = Unsure
        .interpret("like the last good one", &ctx())
        .expect("asking is not an error");
    match &asked {
        Understood::Clarify { question, .. } => assert!(question.ends_with('?')),
        other => panic!("expected a question, got {other:?}"),
    }

    // A question carries no actuation value, so nothing is validated against it
    // and nothing is refused.
    assert!(adi_decision::accept_intent(asked).is_ok());
}

/// Whatever produces an intent, including a future model, its output is checked
/// before the engine acts on it. The engine never assumes a parser got it right.
#[test]
fn an_intent_from_any_source_is_validated_at_the_boundary() {
    use adi_decision::Understood;
    use adi_protocol::intent::{Intent, IntentKind};

    let empty = Understood::Intent(Intent {
        kind: IntentKind::Adjust,
        style: None,
        material: None,
        targets: BTreeMap::new(),
    });
    assert!(matches!(
        adi_decision::accept_intent(empty),
        Err(DecisionError::MalformedSelection(_))
    ));

    let ok = Understood::Intent(Intent {
        kind: IntentKind::Produce,
        style: Some("adi.example.style.standard".into()),
        material: None,
        targets: BTreeMap::new(),
    });
    assert!(adi_decision::accept_intent(ok).is_ok());
}

// -------------------------------------------------- plans across variables

/// Two goals on two variables, and a plan that serves both.
///
/// This is what a request needs: "weaker and smaller than last time" is two
/// goals, and the tools that serve them are different tools. Before `decide`
/// took a set of goals it built one variable's candidate list, so every tool on
/// offer moved the same variable and ADI-P 9.5 rejected any plan longer than one
/// step. The multi-step machinery existed and nothing could reach it.
#[test]
fn a_plan_may_serve_several_goals_at_once() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let both = vec![wants("cycle_time", 4.0), wants("additive_volume", 6.0)];

    // The candidate list now spans both variables.
    let variables: Vec<VariableId> = both
        .iter()
        .filter_map(|g| g.target_variable.clone())
        .collect();
    let candidates = planner.candidates_for_all(&variables);
    assert!(
        candidates
            .iter()
            .any(|c| c.variable == VariableId::from("cycle_time"))
            && candidates
                .iter()
                .any(|c| c.variable == VariableId::from("additive_volume")),
        "both goals must be represented, or the plan cannot serve both"
    );

    let corrections =
        adi_decision::decide(&planner, &EngineSelector, &both, &ctx(), &present()).unwrap();

    assert_eq!(corrections.len(), 2, "one step per goal");
    let moved: Vec<&str> = corrections.iter().map(|c| c.variable.as_str()).collect();
    assert!(moved.contains(&"cycle_time"));
    assert!(moved.contains(&"additive_volume"));
}

/// The same plan at D2, with the model choosing both tools and both magnitudes.
#[test]
fn d2_may_propose_a_magnitude_for_each_goal() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let both = vec![wants("cycle_time", 4.0), wants("additive_volume", 6.0)];
    let variables: Vec<VariableId> = both
        .iter()
        .filter_map(|g| g.target_variable.clone())
        .collect();
    let candidates = planner.candidates_for_all(&variables);

    // Point at one candidate for each variable, whatever order they sorted into.
    let steps: Vec<ProposedStep> = [
        VariableId::from("cycle_time"),
        VariableId::from("additive_volume"),
    ]
    .iter()
    .map(|v| ProposedStep {
        candidate: candidates.iter().position(|c| c.variable == *v).unwrap(),
        value: 20.0,
    })
    .collect();

    let selector = ModelActionSelector::new(FixedPlan(steps), &d).unwrap();
    let corrections = adi_decision::decide(&planner, &selector, &both, &ctx(), &present()).unwrap();

    assert_eq!(corrections.len(), 2);
    for c in &corrections {
        assert!(
            c.absolute.value.is_finite(),
            "every magnitude still goes through the planner"
        );
    }
}

/// The rule that made the old shape unusable still applies, and now it catches
/// only what it was written to catch: two steps reaching one variable.
#[test]
fn two_steps_on_one_variable_are_still_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let selector = ModelActionSelector::new(
        FixedPlan(vec![
            ProposedStep {
                candidate: 0,
                value: 25.0,
            },
            ProposedStep {
                candidate: 1,
                value: 10.0,
            },
        ]),
        &d,
    )
    .unwrap();

    assert!(matches!(
        adi_decision::decide(&planner, &selector, &goals(), &ctx(), &present()),
        Err(DecisionError::ConflictingSteps { .. })
    ));
}

/// A caller asking for one variable twice has asked for a thing and its
/// opposite, and whichever step won would be arbitrary.
#[test]
fn two_goals_on_one_variable_are_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    let same = vec![wants("cycle_time", 4.0), wants("cycle_time", -4.0)];
    assert!(matches!(
        adi_decision::decide(&planner, &EngineSelector, &same, &ctx(), &present()),
        Err(DecisionError::DuplicateGoal(_))
    ));
}

#[test]
fn no_goals_is_refused() {
    let d = declaration();
    let s = skill();
    let domain = TestDomain::new();
    let planner = Planner::new(&d, Some(&s), &domain);

    assert!(matches!(
        adi_decision::decide(&planner, &EngineSelector, &[], &ctx(), &present()),
        Err(DecisionError::NoGoals)
    ));
}

/// D1 must answer every goal it was given. Dropping one silently would leave
/// part of a request unserved with nothing saying so.
#[test]
fn d1_must_answer_every_goal() {
    struct AnswersOnce;
    impl ToolChooser for AnswersOnce {
        fn choose(
            &self,
            _: &[adi_decision::Goal],
            _: &[Candidate],
            _: &SelectionContext<'_>,
        ) -> Result<Vec<usize>, String> {
            Ok(vec![0])
        }
    }

    let selector = ModelToolSelector {
        chooser: AnswersOnce,
    };
    let two = vec![wants("cycle_time", 4.0), wants("additive_volume", 6.0)];
    assert!(matches!(
        selector.select(&two, &candidates(), &ctx()),
        Err(DecisionError::MalformedSelection(_))
    ));
}

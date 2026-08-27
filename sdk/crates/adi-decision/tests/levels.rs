//! Autonomy level behaviour.
//!
//! The two claims worth testing: a model can never select a tool the machine
//! did not declare, and D2 cannot be switched on against a machine that lacks
//! the stateful constraints which make a model-chosen magnitude safe.
//!
//! The fixture is a fictional machine in a fictional domain, so this crate's
//! neutrality is demonstrated rather than asserted.

use adi_core::domain::Diagnosis;
use adi_core::plan::Candidate;
use adi_decision::{
    ActionChooser, ActionSelector, AutonomyLevel, DecisionError, EngineSelector,
    ModelActionSelector, ModelToolSelector, ToolChooser,
};
use adi_protocol::ids::VariableId;
use adi_protocol::{Declaration, SemanticId};

fn fixture() -> Declaration {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/declarations/neutral-machine.yaml");
    let text = std::fs::read_to_string(&path).expect("neutral fixture");
    serde_yaml::from_str(&text).expect("fixture should parse")
}

fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            tool: SemanticId::from("adi.example.actuator.set_position"),
            parameter: "position".into(),
            per_unit: -1.4,
            confidence: 0.75,
        },
        Candidate {
            tool: SemanticId::from("adi.example.process.start"),
            parameter: "stop_at_mass".into(),
            per_unit: -0.4,
            confidence: 0.5,
        },
    ]
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

struct FixedChoice(usize);

impl ToolChooser for FixedChoice {
    fn choose(&self, _: &Diagnosis, _: &[Candidate]) -> Result<usize, String> {
        Ok(self.0)
    }
}

impl ActionChooser for FixedChoice {
    fn choose(&self, _: &Diagnosis, _: &[Candidate]) -> Result<(usize, f64), String> {
        Ok((self.0, 21.0))
    }
}

#[test]
fn d0_takes_the_most_confident_candidate() {
    let selection = EngineSelector
        .select(&diagnosis(), &candidates())
        .expect("a candidate exists");
    assert_eq!(
        selection.tool,
        SemanticId::from("adi.example.actuator.set_position")
    );
    assert!(
        selection.model_value.is_none(),
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
        .select(&diagnosis(), &candidates())
        .expect("index 1 is valid");
    assert_eq!(
        selection.tool,
        SemanticId::from("adi.example.process.start"),
        "the model chose the lower-confidence tool, which is its prerogative"
    );
    assert!(
        selection.model_value.is_none(),
        "D1 hands the model the tool, never the number"
    );
}

/// The model answers with an index into a list the engine built, so an
/// out-of-range answer is caught rather than becoming an undeclared tool call.
#[test]
fn d1_cannot_select_a_tool_that_was_not_offered() {
    let selector = ModelToolSelector {
        chooser: FixedChoice(99),
    };
    let error = selector
        .select(&diagnosis(), &candidates())
        .expect_err("index 99 does not exist");
    assert!(matches!(error, DecisionError::UndeclaredSelection(_)));
}

#[test]
fn d1_reports_no_candidate_rather_than_improvising() {
    let selector = ModelToolSelector {
        chooser: FixedChoice(0),
    };
    let error = selector
        .select(&diagnosis(), &[])
        .expect_err("nothing to choose from");
    assert!(matches!(error, DecisionError::NoCandidate(_)));
}

#[test]
fn d2_refuses_a_machine_without_stateful_constraints() {
    let mut bare = fixture();
    bare.envelope.invariants.clear();
    bare.envelope.rate_limits.clear();

    let error = ModelActionSelector::new(FixedChoice(0), &bare)
        .err()
        .expect("D2 must refuse this machine");
    assert!(matches!(error, DecisionError::StatefulEnvelopeRequired));
}

/// `selector_for` can only build D0, because the model-driven levels need a
/// backend supplied by the caller. It reports that rather than panicking.
#[test]
fn selector_for_builds_d0_and_reports_the_rest() {
    let declaration = fixture();

    assert!(adi_decision::selector_for(AutonomyLevel::D0, &declaration).is_ok());

    let e = adi_decision::selector_for(AutonomyLevel::D1, &declaration)
        .err()
        .expect("D1 needs a backend");
    assert!(matches!(e, DecisionError::NoModelBackend("D1")));

    // On a machine with no stateful envelope, D2 reports the harder failure
    // first: the envelope, not the missing backend.
    let mut bare = fixture();
    bare.envelope.invariants.clear();
    bare.envelope.rate_limits.clear();
    let e = adi_decision::selector_for(AutonomyLevel::D2, &bare)
        .err()
        .expect("D2 must refuse");
    assert!(matches!(e, DecisionError::StatefulEnvelopeRequired));
}

#[test]
fn d2_initialises_when_the_envelope_is_stateful() {
    let declaration = fixture();
    let selector = ModelActionSelector::new(FixedChoice(0), &declaration)
        .expect("the fixture declares invariants and rate limits");
    assert_eq!(selector.level(), AutonomyLevel::D2);

    let selection = selector
        .select(&diagnosis(), &candidates())
        .expect("selection should succeed");
    assert!(
        selection.model_value.is_some(),
        "only D2 carries a model-supplied magnitude"
    );
}

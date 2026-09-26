//! The composition tests.
//!
//! These are the tests that make the Safe Action Protocol claim true rather
//! than merely plausible. Every invocation below is individually valid: in
//! range, within every hard limit, preconditions satisfiable. The sequences are
//! not. Per-call checking cannot catch any of this, which is the entire reason
//! stateful constraints exist.
//!
//! The fixture is a fictional machine in a fictional domain. That is
//! deliberate: if this crate could only be exercised against a real domain's
//! declaration, "domain-neutral" would be a claim rather than a property.

use adi_protocol::invocation::{Invocation, RejectReason};
use adi_protocol::units::{Quantity, Unit};
use adi_protocol::{Declaration, SemanticId};
use adi_safety::{Guard, SafetyState, SafetyViolation};

fn fixture() -> Declaration {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/declarations/neutral-machine.yaml");
    let text = std::fs::read_to_string(&path).expect("neutral fixture");
    serde_yaml::from_str(&text).expect("fixture should parse")
}

fn dispense(ml: f64, id: u32) -> Invocation {
    Invocation::new(SemanticId::from("adi.example.additive.dispense"), id)
        .with_text("substance", "a")
        .with_number("volume", Quantity::new(ml, Unit::Ml))
}

fn run_process(ml: f64, id: u32) -> Invocation {
    Invocation::new(SemanticId::from("adi.example.process.start"), id)
        .with_number("stop_at_volume", Quantity::new(ml, Unit::Ml))
}

#[test]
fn individually_valid_calls_breach_the_container_invariant() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let mut state = SafetyState::new();

    // Capacity is 320 ml. Each call is well inside the tool's own 0 to 300
    // range, so no per-call check can object to any of them.
    for (n, id) in [1u32, 2].iter().enumerate() {
        let call = dispense(150.0, *id);
        guard
            .check(&call, &state)
            .unwrap_or_else(|e| panic!("call {n} should pass: {e}"));
        guard.commit(&call, &mut state);
    }

    let third = dispense(150.0, 3);
    let violation = guard
        .check(&third, &state)
        .expect_err("450 into a 320 container must be refused");

    match violation {
        SafetyViolation::Invariant {
            ref invariant,
            projected,
            limit,
        } => {
            assert_eq!(invariant, "container_capacity");
            assert_eq!(projected, 450.0);
            assert_eq!(limit, 320.0);
        }
        other => panic!("expected an invariant violation, got {other:?}"),
    }
    assert_eq!(violation.reject_reason(), RejectReason::InvariantViolation);
}

#[test]
fn emptying_the_container_resets_the_accumulator() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let mut state = SafetyState::new();

    let first = dispense(300.0, 1);
    guard.check(&first, &state).expect("first fits");
    guard.commit(&first, &mut state);
    assert_eq!(state.accumulated("container_capacity"), 300.0);

    let empty = Invocation::new(SemanticId::from("adi.example.container.empty"), 9);
    guard.commit(&empty, &mut state);
    assert_eq!(
        state.accumulated("container_capacity"),
        0.0,
        "an emptied container starts from zero"
    );

    let next = dispense(300.0, 2);
    guard.check(&next, &state).expect("there is room again");
}

#[test]
fn running_again_too_soon_is_rate_limited() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let mut state = SafetyState::new();

    state.advance_to(0.0);
    let first = run_process(36.0, 1);
    guard.check(&first, &state).expect("first run is fine");
    guard.commit(&first, &mut state);

    // The machine declares 25 s of recovery between runs.
    state.advance_to(10.0);
    let second = run_process(36.0, 2);
    let violation = guard
        .check(&second, &state)
        .expect_err("10 s is inside the declared recovery interval");
    assert!(matches!(violation, SafetyViolation::RateLimited { .. }));
    assert_eq!(violation.reject_reason(), RejectReason::RateLimited);

    state.advance_to(30.0);
    guard
        .check(&second, &state)
        .expect("30 s later the machine has recovered");
}

#[test]
fn out_of_range_argument_is_refused() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let state = SafetyState::new();

    // The input tool accepts 6 to 14 g.
    let call = Invocation::new(SemanticId::from("adi.example.input.by_mass"), 1)
        .with_number("mass", Quantity::new(40.0, Unit::G));
    let violation = guard.check(&call, &state).expect_err("40 is out of range");
    assert_eq!(violation.reject_reason(), RejectReason::OutOfRange);
}

#[test]
fn clamping_happens_before_sending_not_after_arrival() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let tool = SemanticId::from("adi.example.input.by_mass");

    let clamped = guard
        .clamp(&tool, "mass", Quantity::new(40.0, Unit::G))
        .expect("clamp should resolve the parameter");
    assert_eq!(clamped.value, 14.0, "clamped to the declared maximum");

    // The appliance still rejects an unclamped value rather than silently
    // fixing it, so an engine defect stays visible. See PROTOCOL.md 4.6.
    let state = SafetyState::new();
    let unclamped = Invocation::new(tool, 1).with_number("mass", Quantity::new(40.0, Unit::G));
    assert!(guard.check(&unclamped, &state).is_err());
}

#[test]
fn unit_mismatch_is_refused() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let state = SafetyState::new();

    // Right number, wrong unit. A declared range in grams must not accept ml.
    let call = Invocation::new(SemanticId::from("adi.example.input.by_mass"), 1)
        .with_number("mass", Quantity::new(10.0, Unit::Ml));
    let violation = guard.check(&call, &state).expect_err("units must match");
    assert!(matches!(violation, SafetyViolation::UnitMismatch { .. }));
}

#[test]
fn open_interlock_blocks_its_tools() {
    let declaration = fixture();
    let mut guard = Guard::new(&declaration);
    let state = SafetyState::new();

    guard.set_open_interlocks(vec!["supply_present".to_string()]);
    let violation = guard
        .check(&run_process(36.0, 1), &state)
        .expect_err("no supply, no run");
    assert_eq!(violation.reject_reason(), RejectReason::Interlock);
}

#[test]
fn wildcard_interlock_blocks_everything() {
    let declaration = fixture();
    let mut guard = Guard::new(&declaration);
    let state = SafetyState::new();

    guard.set_open_interlocks(vec!["service_door_closed".to_string()]);
    let violation = guard
        .check(&dispense(50.0, 1), &state)
        .expect_err("an open service door blocks all tools");
    assert_eq!(violation.reject_reason(), RejectReason::Interlock);
}

#[test]
fn undeclared_tool_is_refused() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let state = SafetyState::new();

    let call = Invocation::new(SemanticId::from("x.nobody.invented.this"), 1);
    let violation = guard.check(&call, &state).expect_err("tool does not exist");
    assert!(matches!(violation, SafetyViolation::UnknownTool(_)));
}

/// Regression test for a bug found by cross-implementation testing.
///
/// The C SDK refuses to add a quantity to an accumulator denominated in a
/// different unit. This crate did not: it added the raw `f64` and ignored the
/// unit entirely, so grams could be summed into a millilitre total. The two
/// implementations disagreed about whether an action was legal, which is
/// exactly the divergence that would surface in the field as an engine
/// permitting something the appliance refuses.
#[test]
fn an_accumulator_refuses_a_foreign_unit() {
    let declaration = fixture();
    let guard = Guard::new(&declaration);
    let state = SafetyState::new();

    // container_capacity is declared in millilitres. Offer it grams.
    let call = Invocation::new(SemanticId::from("adi.example.additive.dispense"), 1)
        .with_text("substance", "a")
        .with_number("volume", Quantity::new(50.0, Unit::G));

    let violation = guard
        .check(&call, &state)
        .expect_err("grams must not accumulate into a millilitre total");
    assert!(matches!(violation, SafetyViolation::UnitMismatch { .. }));
}

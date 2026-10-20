//! The loop, end to end, against a fictional machine.
//!
//! The domain here is invented, as everywhere in the neutral crates. A cycle
//! is a cycle: in one domain it is an extraction, in another a wash, in
//! another three weeks of a plant growing. Nothing in this crate knows which.

use std::collections::BTreeMap;

use adi_core::{Cycle, Diagnosis, Domain, SubjectiveFeedback};
use adi_decision::AutonomyLevel;
use adi_protocol::declaration::Declaration;
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::knowledge::{Knowledge, ResolvedTargets};
use adi_protocol::invocation::RejectReason;
use adi_protocol::skill::Skill;
use adi_protocol::units::{Quantity, Unit};
use adi_session::{Recorded, Session, SessionError};
use adi_wire::{Message, Outcome};

// ------------------------------------------------------------- the fixtures

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn declaration() -> Declaration {
    serde_yaml::from_str(&fixture("declarations/neutral-machine.yaml")).expect("parses")
}

fn skill() -> Skill {
    serde_yaml::from_str(&fixture("declarations/neutral-machine.skill.yaml")).expect("parses")
}

fn knowledge() -> Knowledge {
    let base: adi_protocol::knowledge::KnowledgeBase =
        serde_yaml::from_str(&fixture("knowledge/neutral-base.yaml")).expect("parses");
    Knowledge::new(base.into_layer()).expect("base loads")
}

/// A profile that wants one variable moved, so the loop has something to do.
struct TestDomain {
    knowledge: Knowledge,
    variables: Vec<VariableId>,
    /// The variable this profile wants moved.
    wants: VariableId,
    /// How far off the cycle is judged to be, in the variable's own unit.
    delta: f64,
}

impl TestDomain {
    fn new(delta: f64) -> Self {
        Self::wanting("cycle_time", delta)
    }

    fn wanting(variable: &str, delta: f64) -> Self {
        Self {
            knowledge: knowledge(),
            variables: vec![
                VariableId::from("cycle_time"),
                VariableId::from("output_ratio"),
            ],
            wants: VariableId::from(variable),
            delta,
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
        if self.delta == 0.0 {
            return Diagnosis {
                cause: "within_target".into(),
                target_variable: None,
                desired_delta: None,
                confidence: 1.0,
                explanation: "nothing to correct".into(),
            };
        }
        Diagnosis {
            cause: "off_target".into(),
            target_variable: Some(self.wants.clone()),
            desired_delta: Some(self.delta),
            confidence: 0.8,
            explanation: "outside the target range".into(),
        }
    }
    /// This profile reads both its variables from declared signals, so the
    /// decision record can say what the machine actually saw. A profile that
    /// does not answer gets a record without measured inputs rather than a
    /// record naming a signal it never read.
    fn signal_for(&self, variable: &VariableId) -> Option<SemanticId> {
        match variable.0.as_str() {
            "cycle_time" => Some(SemanticId::from("adi.example.signal.elapsed")),
            "output_ratio" => Some(SemanticId::from("adi.example.signal.output")),
            _ => None,
        }
    }
    fn feedback_vocabulary(&self) -> &[&str] {
        &["good", "bad"]
    }
}

fn cycle(id: &str, position: f64) -> Cycle {
    let mut settings = BTreeMap::new();
    settings.insert(
        VariableId::from("position"),
        Quantity::new(position, Unit::Step),
    );
    settings.insert(
        VariableId::from("cycle_time"),
        Quantity::new(30.0, Unit::S),
    );

    Cycle {
        cycle_id: id.to_string(),
        subject_id: None,
        operator_id: None,
        settings,
        measured: BTreeMap::new(),
        traces: Vec::new(),
        events: Vec::new(),
        feedback: SubjectiveFeedback::default(),
    }
}

/// An appliance that accepts everything, which is the uninteresting case and
/// therefore the right baseline.
fn agreeable(count: usize) -> Recorded {
    let mut t = Recorded::new();
    for id in 1..=count as u32 {
        t.will_reply(Message::Response {
            id,
            outcome: Outcome::Completed,
            reason: None,
        });
    }
    t
}

// -------------------------------------------------------------- the request
//
// The path a spoken instruction takes once it is structured intent, and the
// one path in this crate that had no test at all. `Session::request` had a
// single caller, the interactive prompt, so nothing ever checked that asking
// for something moved a mechanism. It did not: the planner was handed an empty
// map of present settings and refused every step for want of a position to
// move from.
//
// The conformance runner reaches the same decision by its own route and stops
// one step earlier, at which tool was chosen rather than what value was sent,
// so it passed throughout.

fn asking_for(variable: &str, value: f64, unit: Unit) -> adi_protocol::intent::Intent {
    let mut targets = BTreeMap::new();
    targets.insert(
        VariableId::from(variable),
        adi_protocol::intent::TargetSpec::Absolute(Quantity::new(value, unit)),
    );

    adi_protocol::intent::Intent {
        kind: adi_protocol::intent::IntentKind::Adjust,
        style: None,
        material: None,
        targets,
    }
}

/// The headline capability: a person asks for something and the machine moves.
#[test]
fn a_request_moves_a_mechanism() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let transport = agreeable(4);
    let log = transport.log();
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    // A cycle on target, so nothing is corrected and the only thing the engine
    // takes from it is where the mechanism stands.
    session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");
    assert!(log.invocations().is_empty(), "the cycle itself needed nothing");

    let turn = session
        .request(&asking_for("cycle_time", 25.0, Unit::S), 1_000)
        .expect("a request for something a declared tool can move is servable");

    assert_eq!(turn.performed.len(), 1, "one variable, one action");
    assert_eq!(
        log.invocations().len(),
        1,
        "and it reached the wire, which is the whole point"
    );
}

/// A correction moves the machine, so the next request must plan from where it
/// put it and not from where the last cycle found it.
///
/// Asserted by asking for the same thing twice. Working from the cycle alone,
/// both requests see position 20 and send the identical value; working from
/// what was sent, the second starts nearer the goal and sends something else.
#[test]
fn a_request_plans_from_what_was_last_sent() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let transport = agreeable(8);
    let log = transport.log();
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    let first = session
        .request(&asking_for("cycle_time", 25.0, Unit::S), 1_000)
        .expect("servable");
    let second = session
        .request(&asking_for("cycle_time", 25.0, Unit::S), 2_000)
        .expect("servable");

    let sent = |turn: &adi_session::Turn| -> f64 {
        turn.records[0]
            .arguments
            .get("position")
            .expect("the correction set position")
            .value
    };

    assert_eq!(log.invocations().len(), 2, "both requests acted");
    assert_ne!(
        sent(&first),
        sent(&second),
        "the second request planned from the first one's result, not from the cycle"
    );
}

/// A session that has observed nothing and sent nothing does not know where
/// any mechanism stands, and ADI-P has no message for asking the appliance.
/// Refusing is correct; the refusal has to say so.
#[test]
fn a_request_with_no_known_position_says_what_is_missing() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let mut session = Session::open(
        &d,
        Some(&s),
        &dom,
        AutonomyLevel::D0,
        Box::new(Recorded::new()),
    )
    .expect("opens");

    let err = session
        .request(&asking_for("cycle_time", 25.0, Unit::S), 0)
        .expect_err("nothing is known about any position yet");

    let text = err.to_string();
    assert!(
        text.contains("position"),
        "the refusal should name the parameter: {text}"
    );
    assert!(
        !text.contains("malformed"),
        "and must not blame the model, which was not consulted: {text}"
    );
}

// ----------------------------------------------------------------- the loop

#[test]
fn a_cycle_within_target_sends_nothing() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let mut session = Session::open(
        &d,
        Some(&s),
        &dom,
        AutonomyLevel::D0,
        Box::new(Recorded::new()),
    )
    .expect("opens");

    let turn = session.observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    assert!(turn.nominal, "nothing was wrong");
    assert!(turn.performed.is_empty(), "so nothing should have been sent");
    assert_eq!(session.history().len(), 0);
}

#[test]
fn a_cycle_off_target_produces_a_correction() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(-4.0));
    let transport = agreeable(4);
    let log = transport.log();
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    let turn = session.observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    assert!(!turn.nominal);
    assert_eq!(turn.performed.len(), 1, "one variable, one action");
    assert_eq!(
        log.invocations().len(),
        1,
        "and exactly one invocation reached the wire"
    );
    assert_eq!(session.history().len(), 1, "and it is on the record");
    assert_eq!(session.history()[0].autonomy_level, "D0");
}

/// The gate for this phase, and the reason it exists.
///
/// The memory is what a relative request resolves against and what a model is
/// given to reason with. Nothing wrote it before: it existed, and only the
/// conformance runner ever filled it, by hand, to test something else.
#[test]
fn each_cycle_is_remembered_for_the_next() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let mut session = Session::open(
        &d,
        Some(&s),
        &dom,
        AutonomyLevel::D0,
        Box::new(Recorded::new()),
    )
    .expect("opens");

    for (n, position) in [(1, 20.0), (2, 21.0), (3, 22.0)] {
        session
            .observe(
                cycle(&format!("c{n}"), position),
                "adi.example.style.standard",
                None,
                n as u64 * 60_000,
            )
            .expect("observes");
    }

    use adi_core::Memory;
    assert_eq!(session.memory().len(), 3, "all three are held");

    let last = session.memory().last().expect("there is a last");
    assert_eq!(last.cycle_id, "c3");
    assert_eq!(
        last.setting(&VariableId::from("position")),
        Some(Quantity::new(22.0, Unit::Step)),
        "and it carries the settings a relative request resolves against"
    );

    let recent = session.memory().recent(2);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].cycle_id, "c3", "newest first");
}

// -------------------------------------------------------------- the envelope

/// The other gate. Until this phase adi-safety was 370 lines of working
/// envelope enforcement that nothing called, so the engine checked each value
/// against its parameter range and no invariant, rate limit or resource level
/// at all.
///
/// A container invariant is the case that cannot be caught per call: each
/// dispense is inside its own declared range, and their sum is not.
#[test]
fn a_sequence_that_would_breach_an_invariant_is_refused_by_the_engine() {
    use adi_protocol::invocation::Invocation;
    use adi_safety::{Guard, SafetyState};

    let d = declaration();
    let guard = Guard::new(&d);
    let mut state = SafetyState::new();

    let dispense = |id: u32, ml: f64| {
        Invocation::new(SemanticId::from("adi.example.additive.dispense"), id)
            .with_text("substance", "a")
            .with_number("volume", Quantity::new(ml, Unit::Ml))
    };

    // The container holds 320 ml. Each of these is well inside the tool's own
    // 0 to 300 range, so no per-call check can object to any of them.
    for id in 1..=2u32 {
        let call = dispense(id, 150.0);
        guard.check(&call, &state).expect("each call is individually valid");
        guard.commit(&call, &mut state);
    }

    let third = dispense(3, 150.0);
    let refused = guard
        .check(&third, &state)
        .expect_err("450 into a 320 container must be refused");

    assert!(
        format!("{refused}").contains("container") || format!("{refused:?}").contains("Invariant"),
        "the refusal should name the invariant, got {refused:?}"
    );
}

/// The gate this phase exists for.
///
/// adi-safety has been complete and tested since the beginning and nothing
/// called it, so the engine clamped each value to its parameter range and
/// checked no invariant at all. The appliance caught everything, which is why
/// nothing ever broke and why it still mattered: under ADI-P 9.2 a refused
/// invocation is recorded as an engine defect, and the manufacturer counting
/// those is counting ours.
///
/// process.start is the only tool that moves output_ratio, and its
/// stop_at_volume accumulates into a 320 ml container. Each call clamps to the
/// parameter's 90 ml maximum, so three fit and a fourth cannot. No per-call
/// check can see that coming: every one of them is individually valid.
#[test]
fn the_engine_refuses_a_sequence_that_would_breach_the_container() {
    let (d, s) = (declaration(), skill());
    let dom = TestDomain::wanting("output_ratio", 2.0);

    let transport = agreeable(8);
    let log = transport.log();
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    let filling = |n: u32| {
        let mut c = cycle(&format!("c{n}"), 20.0);
        c.settings.insert(
            VariableId::from("stop_at_volume"),
            Quantity::new(15.0, Unit::Ml),
        );
        c
    };

    for n in 1..=3u32 {
        session
            .observe(filling(n), "adi.example.style.standard", None, n as u64 * 60_000)
            .unwrap_or_else(|e| panic!("cycle {n} should be within the container: {e}"));
    }

    assert_eq!(log.invocations().len(), 3, "three went out");

    let refused = session
        .observe(filling(4), "adi.example.style.standard", None, 4 * 60_000)
        .expect_err("the fourth would overflow a 320 ml container");

    assert!(
        matches!(refused, SessionError::Envelope(_)),
        "it must be the envelope that refuses, got {refused:?}"
    );

    assert_eq!(
        log.invocations().len(),
        3,
        "and the refused call must not have reached the wire"
    );
}

// ------------------------------------------------------------- the heartbeat

/// ADI-P 9.1 requirement 4. This is the message that did not exist until
/// phase 2, and without it every operation longer than control_timeout_ms
/// aborted itself.
#[test]
fn the_session_heartbeats_on_its_declared_interval() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));
    let interval = d.session.heartbeat_interval_ms;

    let transport = Recorded::new();
    let log = transport.log();

    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    // Well inside the interval: nothing to say yet.
    session.tick(interval / 2).expect("ticks");
    assert_eq!(log.count(adi_wire::Kind::Heartbeat), 0, "too early to speak");

    session.tick(interval + 1).expect("ticks");
    assert_eq!(log.count(adi_wire::Kind::Heartbeat), 1, "one is due");

    // Two more ticks inside the next interval must not produce two more. A
    // session sending one per tick would flood a UART that has real work to
    // carry.
    session.tick(interval + 2).expect("ticks");
    session.tick(interval + 3).expect("ticks");
    assert_eq!(log.count(adi_wire::Kind::Heartbeat), 1, "still only one");

    session.tick(interval * 2 + 4).expect("ticks");
    assert_eq!(log.count(adi_wire::Kind::Heartbeat), 2, "the next is due");
}

// ----------------------------------------------------------------- refusals

#[test]
fn a_refusal_from_the_appliance_stops_the_turn() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(-4.0));

    let mut transport = Recorded::new();
    transport.will_reply(Message::Response {
        id: 1,
        outcome: Outcome::Rejected,
        reason: Some(RejectReason::Interlock),
    });

    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(transport))
        .expect("opens");

    let err = session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect_err("a refused call is not a successful turn");

    assert!(
        matches!(err, SessionError::Refused(RejectReason::Interlock)),
        "the reason should survive, got {err:?}"
    );
}

#[test]
fn silence_from_the_appliance_is_an_error_not_a_success() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(-4.0));

    // A transport with nothing queued: the appliance never answers.
    let mut session = Session::open(
        &d,
        Some(&s),
        &dom,
        AutonomyLevel::D0,
        Box::new(Recorded::new()),
    )
    .expect("opens");

    let err = session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect_err("an unanswered call must not read as done");

    assert!(matches!(err, SessionError::Silent), "got {err:?}");
}

/// ADI-P 3.3: an engine must not quietly run at a level other than the one it
/// was asked for, because a record naming the wrong level is false and the
/// record is what the certification argument rests on.
#[test]
fn an_unavailable_level_refuses_to_open_rather_than_falling_back() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(0.0));

    let opened = Session::open(
        &d,
        Some(&s),
        &dom,
        AutonomyLevel::D1,
        Box::new(Recorded::new()),
    );

    assert!(
        opened.is_err(),
        "D1 has no model backend, so the session must not open at D0 instead"
    );
}

// ------------------------------------------------- the record, ADI-P 9.3.5

// ADI-P 9.3 requirement 5: a decision must be reconstructable from its
// record. These assert the fields are populated, not merely present. Every one
// of them existed, typed and tested, and the engine wrote nothing into them.

/// `cause` must hold the diagnosed cause, and it held the variable the
/// correction moves. The difference is the difference between "the shot
/// channeled" and "the grind changed": one is the reason and the other is the
/// consequence, and a record carrying only the consequence answers nothing.
#[test]
fn a_record_states_why_rather_than_what() {
    let (d, s, dom) = (declaration(), skill(), TestDomain::new(-4.0));
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(agreeable(4)))
        .expect("opens");

    session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    let record = &session.history()[0];
    assert_eq!(record.cause, "off_target", "the diagnosed cause");
    assert_ne!(
        record.cause, "cycle_time",
        "not the variable the correction moves"
    );
    assert_eq!(record.rule_id, "off_target", "the rule that produced it");
    assert_eq!(
        record.explanation, "outside the target range",
        "the profile composes this on every diagnosis and it was discarded"
    );
}

/// Requirement 5 again, on the part the specification is most explicit about:
/// "the dose came from the vendor default, the temperature from the user's
/// learned preference" is the level of detail required, so each value carries
/// its own layer and its own kind.
#[test]
fn a_record_accounts_for_every_value_that_fed_it() {
    use adi_core::ValueSource;

    let (d, s, dom) = (declaration(), skill(), TestDomain::new(-4.0));
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(agreeable(4)))
        .expect("opens");

    session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    let inputs = &session.history()[0].inputs;
    assert!(!inputs.is_empty(), "the record must account for its inputs");

    // What the machine saw.
    let measured: Vec<_> = inputs
        .iter()
        .filter(|i| matches!(i.source, ValueSource::Measured { .. }))
        .collect();
    assert_eq!(measured.len(), 1, "one measured value fed this decision");
    assert_eq!(measured[0].value.value, 30.0);

    // What its knowledge said to aim at, and which layer said it.
    let from_knowledge: Vec<_> = inputs
        .iter()
        .filter(|i| matches!(i.source, ValueSource::Knowledge { .. }))
        .collect();
    assert_eq!(from_knowledge.len(), 1);
    assert_eq!(
        from_knowledge[0].value.value, 26.0,
        "observed 30 and wanted 4 less, so the engine was aiming at 26"
    );
    assert_eq!(from_knowledge[0].layer, adi_protocol::knowledge::Layer::Base);

    // What the manufacturer declared the mechanism does. Without this the
    // arithmetic cannot be replayed: the step is the desired change divided
    // by it.
    let from_skill: Vec<_> = inputs
        .iter()
        .filter(|i| matches!(i.source, ValueSource::SkillResponse { .. }))
        .collect();
    assert_eq!(from_skill.len(), 1);
    assert_ne!(from_skill[0].value.value, 0.0);
}

/// A profile that cannot say which signal a variable came from gets a record
/// without measured inputs. Naming a signal that was never read would put a
/// measurement in the record that was never taken, and the record is read when
/// something has already gone wrong.
#[test]
fn a_silent_profile_records_no_measurement_rather_than_a_guess() {
    use adi_core::ValueSource;

    struct Quiet(TestDomain);
    impl Domain for Quiet {
        fn profile_id(&self) -> &str {
            self.0.profile_id()
        }
        fn variables(&self) -> &[VariableId] {
            self.0.variables()
        }
        fn knowledge(&self) -> &Knowledge {
            self.0.knowledge()
        }
        fn derive(&self, c: &mut Cycle) {
            self.0.derive(c)
        }
        fn diagnose(&self, c: &Cycle, t: &ResolvedTargets) -> Diagnosis {
            self.0.diagnose(c, t)
        }
        fn feedback_vocabulary(&self) -> &[&str] {
            self.0.feedback_vocabulary()
        }
        // signal_for deliberately not implemented: the trait default is None.
    }

    let (d, s) = (declaration(), skill());
    let dom = Quiet(TestDomain::new(-4.0));
    let mut session = Session::open(&d, Some(&s), &dom, AutonomyLevel::D0, Box::new(agreeable(4)))
        .expect("opens");

    session
        .observe(cycle("c1", 20.0), "adi.example.style.standard", None, 0)
        .expect("observes");

    let inputs = &session.history()[0].inputs;
    assert!(
        !inputs
            .iter()
            .any(|i| matches!(i.source, ValueSource::Measured { .. })),
        "no signal was declared, so nothing may be recorded as measured"
    );
    assert!(
        inputs
            .iter()
            .any(|i| matches!(i.source, ValueSource::SkillResponse { .. })),
        "the rest of the account is unaffected"
    );
}

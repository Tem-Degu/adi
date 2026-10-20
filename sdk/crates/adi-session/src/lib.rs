//! The engine loop.
//!
//! Every piece of the reasoning has existed as a library for some time, and
//! nothing owned the order they run in. The conformance runner assembled a
//! partial version to test against, and that was the only place a cycle was
//! ever carried from observation to invocation. This is the part that turns a
//! set of crates into something you can point at a machine.
//!
//! It owns what has to outlive one decision: the declaration, the skill and
//! the resolved knowledge; the safety state, because an envelope that only
//! exists within a call cannot constrain a sequence; what the machine
//! remembers producing; the decision history; the link; and the heartbeat.
//!
//! This is also where `adi-safety` is finally called. It has been complete and
//! tested from the start and appeared in the engine only as an error variant,
//! so until now the engine clamped each value to its parameter range and
//! checked no invariant, no rate limit and no resource level. The appliance
//! caught all of it, which is why nothing broke and why it still mattered:
//! under ADI-P 9.2 a refused invocation is recorded as an engine defect, and a
//! manufacturer counting those is counting ours.

pub mod child;
pub mod transport;

use std::collections::BTreeMap;

use adi_core::{Cycle, DecisionRecord, Domain, Memory, Planner, RecentCycles, Remembered};
use adi_decision::{decide, selector_for, ActionSelector, AutonomyLevel, SelectionContext};
use adi_protocol::declaration::Declaration;
use adi_protocol::intent::Intent;
use adi_protocol::invocation::{Invocation, RejectReason};
use adi_protocol::skill::Skill;
use adi_protocol::units::Quantity;
use adi_safety::{Guard, SafetyState};
use adi_wire::{Message, Outcome};

pub use child::ChildAppliance;
pub use transport::{Recorded, Transport};

/// What a set of corrections was decided from, carried to the record.
///
/// ADI-P 9.3 requirement 5 obliges the engine to retain the inputs to every
/// decision. All of this was in scope where the plan was made and went out of
/// scope before it was recorded, which is why the records held an empty input
/// list while every value in them was correct.
///
/// Borrowed rather than cloned: it lives for one call, between deciding and
/// recording, and a decision record that cost a copy of the cycle would be a
/// reason to stop writing them.
struct Grounds<'a> {
    /// The diagnoses the plan answers, one per goal.
    diagnoses: &'a [adi_core::Diagnosis],
    /// The observed cycle, on the observation path. A request has none, which
    /// is why the measured entries are absent there rather than empty.
    cycle: Option<&'a Cycle>,
    /// The resolved knowledge, which carries the layer behind each target.
    resolution: Option<&'a adi_protocol::knowledge::Resolution>,
    /// True on the request path. A value the person asked for did not come
    /// from knowledge, and recording it as though it had would misattribute
    /// the decision to the manufacturer.
    operator_asked: bool,
}

impl<'a> Grounds<'a> {
    /// The diagnosis that asked for this variable to move.
    ///
    /// Matched by variable rather than by position: a plan can serve several
    /// goals, and the selector is free to order its steps as it likes, so
    /// zipping the two lists would attach the wrong reason to the right
    /// action. That is a failure the record exists to prevent.
    fn diagnosis_for(&self, variable: &adi_protocol::ids::VariableId) -> Option<&adi_core::Diagnosis> {
        self.diagnoses
            .iter()
            .find(|d| d.target_variable.as_ref() == Some(variable))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the link failed: {0}")]
    Link(String),
    #[error("the appliance refused the call: {0:?}")]
    Refused(RejectReason),
    #[error("the appliance went quiet")]
    Silent,
    #[error(transparent)]
    Decision(#[from] adi_decision::DecisionError),
    #[error(transparent)]
    Core(#[from] adi_core::CoreError),
    #[error(transparent)]
    Wire(#[from] adi_wire::WireError),
    #[error("the plan breaches the envelope and none of it was performed: {0}")]
    Envelope(#[from] adi_safety::SafetyViolation),
    #[error(transparent)]
    Request(#[from] adi_core::RequestError),
    #[error("knowledge: {0}")]
    Knowledge(String),
}

/// What one turn of the loop produced.
#[derive(Debug, Default)]
pub struct Turn {
    /// What was decided, with the layer behind every value.
    pub records: Vec<DecisionRecord>,
    /// What was actually sent and accepted.
    pub performed: Vec<Invocation>,
    /// True when nothing was wrong and nothing needed doing.
    pub nominal: bool,
}

/// One period of engine control over one appliance. ADI-P 9.1.
pub struct Session<'a> {
    declaration: &'a Declaration,
    skill: Option<&'a Skill>,
    domain: &'a dyn Domain,

    level: AutonomyLevel,
    selector: Box<dyn ActionSelector>,

    guard: Guard<'a>,
    safety: SafetyState,

    memory: RecentCycles,
    history: Vec<DecisionRecord>,

    /// What each mechanism is believed to be set to, keyed by parameter name.
    ///
    /// Two writers, in whatever order they happen, and the later one stands.
    /// The engine writes what it sent, which is provisional: the appliance
    /// accepted it, so it is the best belief available until the appliance
    /// says otherwise. The appliance writes what it reports, which is fact.
    ///
    /// ADI-P 9.1 requires that an operator can take control by physical means
    /// at any moment, so a value the engine merely remembers is stale by
    /// mandate and a machine that reports its own state is the only way the
    /// engine can be right about one it did not set.
    believed: BTreeMap<String, Quantity>,

    /// The latest reading of each outcome the appliance measures.
    ///
    /// A correction is the difference between where an outcome is and where it
    /// should be, so without this the engine can size nothing: a request for a
    /// change computed a change of zero, which an appliance accepts and which
    /// moves no mechanism. The appliance publishes these as it measures them;
    /// a signal that declares `measures` is what says which outcome a reading
    /// belongs to.
    ///
    /// The latest reading only. A series belongs to a cycle, which is observed
    /// as a whole and diagnosed by the profile; this is the present state of
    /// the machine, which is what a request arriving now is about.
    measured: BTreeMap<adi_protocol::ids::VariableId, Quantity>,

    transport: Box<dyn Transport>,

    /// Whether the appliance answered the last heartbeat.
    ///
    /// A port that opens is not evidence of a machine, and until this existed
    /// nothing could tell the difference: `tick` sent a heartbeat and
    /// discarded whatever came back, so a status check reported a live link
    /// with nothing on the other end. That cost a session of work against
    /// firmware which, it turned out, had never started.
    ///
    /// An appliance is not obliged to answer, so this is reported rather than
    /// treated as a fault. What it supports is a truthful answer to the
    /// question a person is actually asking, which is whether anything is
    /// listening.
    answered_last_tick: bool,

    next_invocation_id: u32,
    now_ms: u64,
    last_heartbeat_ms: u64,
    heartbeat_interval_ms: u64,
    control_timeout_ms: u64,
}

impl<'a> Session<'a> {
    /// Open a session.
    ///
    /// Fails when the requested autonomy level cannot be honoured, rather than
    /// quietly running at a lower one. ADI-P 3.3 is explicit: a record naming a
    /// level other than the one that ran is false, and the record is what the
    /// certification argument rests on.
    pub fn open(
        declaration: &'a Declaration,
        skill: Option<&'a Skill>,
        domain: &'a dyn Domain,
        level: AutonomyLevel,
        transport: Box<dyn Transport>,
    ) -> Result<Self, SessionError> {
        let selector = selector_for(level, declaration)?;
        Ok(Self::with_parts(declaration, skill, domain, level, selector, transport))
    }

    /// Open a session with a selector built by the caller.
    ///
    /// The model-driven levels need a chooser, which `selector_for` has no way
    /// to supply, so `open` can only ever build D0. This is how D1 and D2 are
    /// reached, and until it existed neither level could run outside a unit
    /// test however complete the selectors were.
    ///
    /// The level is taken from the selector rather than passed separately, so
    /// a record cannot name a level the selector does not implement.
    pub fn open_with_selector(
        declaration: &'a Declaration,
        skill: Option<&'a Skill>,
        domain: &'a dyn Domain,
        selector: Box<dyn ActionSelector>,
        transport: Box<dyn Transport>,
    ) -> Self {
        let level = selector.level();
        Self::with_parts(declaration, skill, domain, level, selector, transport)
    }

    fn with_parts(
        declaration: &'a Declaration,
        skill: Option<&'a Skill>,
        domain: &'a dyn Domain,
        level: AutonomyLevel,
        selector: Box<dyn ActionSelector>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            declaration,
            skill,
            domain,
            level,
            selector,
            guard: Guard::new(declaration),
            safety: SafetyState::new(),
            memory: RecentCycles::default(),
            history: Vec::new(),
            believed: BTreeMap::new(),
            measured: BTreeMap::new(),
            answered_last_tick: false,
            transport,
            next_invocation_id: 1,
            now_ms: 0,
            last_heartbeat_ms: 0,
            heartbeat_interval_ms: declaration.session.heartbeat_interval_ms,
            control_timeout_ms: declaration.session.control_timeout_ms,
        }
    }

    pub fn level(&self) -> AutonomyLevel {
        self.level
    }

    pub fn history(&self) -> &[DecisionRecord] {
        &self.history
    }

    pub fn memory(&self) -> &RecentCycles {
        &self.memory
    }

    /// Keep the link alive. ADI-P 9.1 requirement 4.
    ///
    /// Call this from whatever loop the host already has. An engine that has
    /// issued a command and is waiting for its result has nothing to say, and
    /// silence is what the appliance's failsafe acts on, so saying nothing is
    /// not an option available to us.
    pub fn tick(&mut self, now_ms: u64) -> Result<(), SessionError> {
        self.advance_to(now_ms);

        if now_ms.saturating_sub(self.last_heartbeat_ms) < self.heartbeat_interval_ms {
            return Ok(());
        }

        self.transport.send(&Message::Heartbeat {
            busy: None,
            faulted: None,
        })?;
        self.last_heartbeat_ms = now_ms;

        // The reply carries busy and faulted. Absent is not an error here: a
        // heartbeat is a liveness probe, and the timeout that matters belongs
        // to the appliance.
        //
        // Everything waiting, not the first thing waiting. An appliance that
        // has something to say sends it when it happens rather than when the
        // engine asks, so by the time a heartbeat goes out there may be a
        // reply, a batch of samples and a setpoint report queued behind each
        // other. Taking one per heartbeat leaves the rest to arrive a second
        // late each, and the report is the one that cannot wait: a request
        // arriving before it is read is refused for want of a position the
        // appliance has already given.
        //
        // Bounded, because a machine that talks faster than this loop drains
        // would otherwise hold it here. Anything still queued is read on the
        // next heartbeat.
        // The first read waits as long as an invocation would, because this is
        // where an appliance's answer to the heartbeat arrives and a reply
        // that crosses an emulated UART does not come back inside fifty
        // milliseconds. Later reads only drain what is already there.
        let mut wait = self.control_timeout_ms.min(500) as u32;
        let mut heard = false;
        for _ in 0..64 {
            match self.transport.recv(wait)? {
                Some(message) => {
                    heard = true;
                    self.absorb(&message);
                }
                None => break,
            }
            wait = self.heartbeat_interval_ms.min(50) as u32;
        }
        self.answered_last_tick = heard;
        Ok(())
    }

    /// Ask the appliance to describe itself, and wait for the answer.
    ///
    /// ADI-P 9.1 lists five steps for a session: fetch the declaration,
    /// validate, open, heartbeat, close. None of them establishes what the
    /// machine is currently set to, and without that the engine cannot plan a
    /// correction: it knows how far a mechanism should move and not what to
    /// send to move it there.
    ///
    /// A conforming appliance reports its setpoints on first contact, so this
    /// heartbeats and reads what comes back. It is bounded and it is allowed
    /// to learn nothing: an appliance that does not report is not faulty, it
    /// is one whose positions the engine will have to be told another way.
    ///
    /// Called once after opening. Without it the first request races the
    /// report and fails about two times in three, which is worse than failing
    /// every time: a machine that works on the second attempt teaches the
    /// person using it that the first one does not count.
    pub fn synchronise(&mut self, now_ms: u64) -> Result<(), SessionError> {
        for attempt in 0..3u32 {
            // Each round is a fresh heartbeat, because the appliance reports
            // on contact and one lost heartbeat should not end the matter.
            self.last_heartbeat_ms = 0;
            self.tick(now_ms + u64::from(attempt) * self.heartbeat_interval_ms)?;

            if !self.believed.is_empty() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Observe a finished cycle and correct if something was wrong.
    ///
    /// The correction path: derive, resolve targets through the knowledge
    /// layers, diagnose, decide, check the envelope, invoke, record, remember.
    /// `now_ms` is passed in rather than read from a clock, for the same
    /// reason `adip_check_stateful` takes it on the appliance side: a rate
    /// limit tested against one instant and recorded against another is a
    /// defect in safety code, and a caller replaying a sequence must get the
    /// same answers.
    pub fn observe(
        &mut self,
        mut cycle: Cycle,
        style: &str,
        material: Option<&str>,
        now_ms: u64,
    ) -> Result<Turn, SessionError> {
        self.advance_to(now_ms);
        self.domain.derive(&mut cycle);

        let resolution = self
            .domain
            .knowledge()
            .resolve(style, material)
            .map_err(|e| SessionError::Knowledge(e.to_string()))?;

        let diagnosis = self.domain.diagnose(&cycle, &resolution.targets);

        // Remembered before any correction is planned, because what the
        // machine just produced is true whether or not it was good, and a
        // relative request that arrives next needs it either way.
        self.remember(&cycle, style);

        // No variable named means no actuation will help: either nothing is
        // wrong, or the fault is one a person has to fix. Either way the
        // engine has nothing to send.
        if diagnosis.target_variable.is_none() {
            return Ok(Turn {
                nominal: true,
                ..Default::default()
            });
        }

        let current = self.current_settings(&cycle.settings);
        let planner = Planner::new(self.declaration, self.skill, self.domain);
        let context = SelectionContext {
            history: &self.history,
            cycle: Some(&cycle),
            memory: Some(&self.memory),
            rejected: None,
            settings: None,
            measured: None,
        };

        let corrections = decide(
            &planner,
            self.selector.as_ref(),
            std::slice::from_ref(&diagnosis),
            &context,
            &current,
        )?;

        self.perform(
            corrections,
            &cycle.cycle_id,
            &Grounds {
                diagnoses: std::slice::from_ref(&diagnosis),
                cycle: Some(&cycle),
                resolution: Some(&resolution),
                operator_asked: false,
            },
        )
    }

    /// Serve a request. The path a spoken instruction takes once it has become
    /// structured intent.
    pub fn request(&mut self, intent: &Intent, now_ms: u64) -> Result<Turn, SessionError> {
        self.advance_to(now_ms);
        let resolved = adi_core::request::resolve(intent, &self.memory)?;
        let knowledge = self.domain.knowledge();

        let mut goals = Vec::new();
        for (variable, target) in &resolved.targets {
            // What the outcome is now, which is what makes the difference
            // between a target and a correction. A reading the appliance has
            // just published outranks the last cycle: the cycle is what the
            // machine did, and the reading is where it stands.
            let present = self
                .measured
                .get(variable)
                .copied()
                .or_else(|| self.memory.last().and_then(|r| r.setting(variable)));

            goals.push(adi_core::request::as_diagnosis(
                variable,
                *target,
                present,
                knowledge,
            ));
        }

        if goals.is_empty() {
            return Ok(Turn {
                nominal: true,
                ..Default::default()
            });
        }

        let planner = Planner::new(self.declaration, self.skill, self.domain);
        let context = SelectionContext {
            history: &self.history,
            cycle: None,
            memory: Some(&self.memory),
            rejected: None,
            settings: None,
            measured: None,
        };

        let corrections = decide(
            &planner,
            self.selector.as_ref(),
            &goals,
            &context,
            &self.present_settings(),
        )?;

        self.perform(
            corrections,
            "request",
            &Grounds {
                diagnoses: &goals,
                // A request is served without observing a cycle, so there is
                // nothing measured to attribute. The engine knows what it was
                // asked for and not what the machine last did.
                cycle: None,
                resolution: None,
                operator_asked: true,
            },
        )
    }

    /// Take whatever the appliance says about itself out of a message.
    ///
    /// Called wherever the session reads from the wire, because a report
    /// arrives when the appliance has something to say rather than when the
    /// engine happens to be listening for it. Both read sites previously
    /// discarded everything that was not the message they were waiting for.
    ///
    /// Anything else is ignored here rather than refused: a message this
    /// version does not act on is not a protocol error, and an engine that
    /// failed on one could not talk to a newer appliance.
    fn absorb(&mut self, message: &Message) {
        match message {
            Message::Setpoints(reported) => {
                for setpoint in reported {
                    let Some(name) = self.parameter_name(setpoint.tool, setpoint.parameter) else {
                        // An index this declaration does not have. The
                        // appliance is describing a machine the engine was not
                        // given, which is a mismatched declaration rather than
                        // a value to act on.
                        continue;
                    };
                    self.believed
                        .insert(name, adi_wire::to_quantity(setpoint.value));
                }
            }
            Message::Signals(samples) => {
                for sample in samples {
                    let Some((variable, value)) = self.measurement(sample) else {
                        continue;
                    };
                    self.measured.insert(variable, value);
                }
            }
            _ => {}
        }
    }

    /// What one sample measures, where the declaration says it measures
    /// anything.
    ///
    /// Three ways this returns nothing, and each is a reading the engine must
    /// not plan from. An index the declaration does not have describes a
    /// machine the engine was not given. A signal with no `measures` is
    /// telemetry: worth publishing, and not an outcome anyone states a target
    /// on. A value in the wrong unit would be compared against a target in
    /// another, and arithmetic across units yields a number that means nothing
    /// and would be sent to hardware anyway.
    fn measurement(
        &self,
        sample: &adi_wire::Sample,
    ) -> Option<(adi_protocol::ids::VariableId, Quantity)> {
        let signal = self.declaration.signals.get(usize::from(sample.signal))?;
        let measures = signal.measures.as_ref()?;
        let value = adi_wire::to_quantity(sample.value);
        if value.unit != signal.unit {
            return None;
        }
        Some((
            adi_protocol::ids::VariableId(measures.0.clone()),
            value,
        ))
    }

    /// Whether anything answered the last heartbeat.
    ///
    /// False is not a fault: an appliance may say nothing, and the failsafe
    /// that matters is the appliance's own. It does mean the engine has no
    /// evidence that a machine is there, which is the difference between a
    /// port that opened and a machine that is listening.
    pub fn answered_last_tick(&self) -> bool {
        self.answered_last_tick
    }

    /// The latest reading of each outcome the appliance measures.
    pub fn measurements(&self) -> &BTreeMap<adi_protocol::ids::VariableId, Quantity> {
        &self.measured
    }

    /// The parameter an index pair names, by the order both sides read from
    /// the declaration: tools in the order written, parameters sorted by name
    /// within each. `adi generate-c` emits the appliance's tables from the
    /// same two orderings, so an index means one thing on both sides.
    fn parameter_name(&self, tool: u16, parameter: u16) -> Option<String> {
        self.declaration
            .tools
            .get(usize::from(tool))?
            .parameters
            .keys()
            .nth(usize::from(parameter))
            .cloned()
    }

    /// Move the envelope's clock forward.
    ///
    /// Rate limits and duty windows are the only part of the state that
    /// depends on time, and they are the part that cannot be judged from one
    /// call. Without this every cycle in a session looks simultaneous, and a
    /// minimum interval between operations is breached on the second one.
    fn advance_to(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.safety.advance_to(now_ms as f64 / 1000.0);
    }

    // ------------------------------------------------------------- internals

    /// Check the whole plan, then send it.
    ///
    /// The envelope is checked across every step before any of them is sent,
    /// and a failure refuses the plan whole. ADI-P 9.3 requirement 2 says so
    /// for a model's magnitudes, and the reason generalises: three steps that
    /// are each in range can still overflow a container, and finding that out
    /// after the first two have already poured is not finding it out.
    fn perform(
        &mut self,
        corrections: Vec<adi_core::Correction>,
        cycle_id: &str,
        grounds: &Grounds<'_>,
    ) -> Result<Turn, SessionError> {
        let mut invocations = Vec::new();
        for correction in &corrections {
            let id = self.next_invocation_id;
            self.next_invocation_id = self.next_invocation_id.wrapping_add(1).max(1);

            invocations.push(
                Invocation::new(correction.tool.clone(), id)
                    .with_number(&correction.parameter, correction.absolute),
            );
        }

        // Every step against the envelope, with the state advancing as it
        // would, so a sequence is judged as a sequence.
        let mut projected = self.safety.clone();
        for invocation in &invocations {
            self.guard.check(invocation, &projected)?;
            self.guard.commit(invocation, &mut projected);
        }

        // Only now is anything sent, and only now does the real state move.
        let mut performed = Vec::new();
        for invocation in &invocations {
            self.send_and_confirm(invocation)?;
            self.guard.commit(invocation, &mut self.safety);
            performed.push(invocation.clone());
        }

        // Provisional, and superseded by the appliance's own report when one
        // arrives. Written here because the invocation was accepted, so this
        // is the best the engine can believe until it is told otherwise.
        for correction in &corrections {
            self.believed
                .insert(correction.parameter.clone(), correction.absolute);
        }

        let records = self.record(&corrections, &invocations, cycle_id, grounds);
        self.history.extend(records.iter().cloned());

        Ok(Turn {
            records,
            performed,
            nominal: false,
        })
    }

    fn send_and_confirm(&mut self, invocation: &Invocation) -> Result<(), SessionError> {
        self.transport
            .send(&adi_wire::invocation(invocation)?)?;

        // Accept first, then whatever the operation finishes as. Telemetry
        // arriving in between is not the answer to this question.
        let mut accepted = false;
        for _ in 0..512 {
            match self.transport.recv(self.control_timeout_ms as u32)? {
                Some(Message::Response { outcome, reason, .. }) => match outcome {
                    Outcome::Rejected => {
                        return Err(SessionError::Refused(
                            reason.unwrap_or(RejectReason::Fault),
                        ))
                    }
                    Outcome::Aborted => {
                        return Err(SessionError::Refused(
                            reason.unwrap_or(RejectReason::Fault),
                        ))
                    }
                    Outcome::Accepted | Outcome::Running => accepted = true,
                    Outcome::Completed => return Ok(()),
                },
                Some(other) => {
                    // Not the answer to this question, and not rubbish
                    // either: an appliance that has just acted reports its
                    // new setting, and that report arrives here.
                    self.absorb(&other);
                    continue;
                }
                None => break,
            }
        }

        if accepted {
            // Accepted but not yet finished. The caller owns what happens
            // next; the envelope has already been committed, because the
            // machine is occupied and the material is consumed from the moment
            // it starts.
            return Ok(());
        }

        Err(SessionError::Silent)
    }

    /// Account for a set of corrections. ADI-P 9.3 requirements 5 and 6.
    ///
    /// Requirement 5 is the demanding one: the record must hold the inputs, the
    /// knowledge layer behind each value used, and the rule that produced the
    /// decision, in enough detail to reconstruct it. Everything it needs was
    /// already computed by the time the plan was sent, and was being discarded
    /// here.
    fn record(
        &self,
        corrections: &[adi_core::Correction],
        invocations: &[Invocation],
        cycle_id: &str,
        grounds: &Grounds<'_>,
    ) -> Vec<DecisionRecord> {
        corrections
            .iter()
            .zip(invocations)
            .enumerate()
            .map(|(n, (correction, invocation))| {
                let diagnosis = grounds.diagnosis_for(&correction.variable);

                DecisionRecord {
                    cycle_id: cycle_id.to_string(),
                    decision_id: format!("{cycle_id}/{n}"),
                    autonomy_level: self.level.as_str(),
                    // The diagnosed cause, which is what 9.3 asks for. This
                    // field held the variable the correction moves, so the
                    // record of a channeling shot read "grind_setting": the
                    // thing changed rather than the reason for changing it,
                    // and the reason is the entire point of the field.
                    cause: diagnosis
                        .map(|d| d.cause.clone())
                        .unwrap_or_else(|| correction.variable.0.clone()),
                    // This profile identifies each rule by the cause it
                    // produces: one rule, one cause, so these are the same
                    // string rather than one standing in for the other. A
                    // profile whose rules and causes diverge gives `Diagnosis`
                    // an identifier of its own, and until one exists adding
                    // that field would be designing from no examples.
                    rule_id: diagnosis.map(|d| d.cause.clone()).unwrap_or_default(),
                    selected_tool: Some(correction.tool.clone()),
                    selected_by: self.level.selected_by(),
                    inputs: self.provenance(correction, grounds),
                    arguments: invocation.numeric_arguments.clone(),
                    // Recorded only where the declared range would not allow
                    // what the engine wanted. Without this the record says the
                    // machine acted, and nothing distinguishes a small
                    // correction from a mechanism at its limit that cannot
                    // move.
                    clamped_from: correction
                        .clamped_from
                        .map(|wanted| {
                            BTreeMap::from([(correction.parameter.clone(), wanted.value)])
                        })
                        .unwrap_or_default(),
                    // Composed by the profile on every diagnosis and discarded
                    // here until now. It is what the language layer phrases,
                    // and never an actuation input.
                    explanation: diagnosis
                        .map(|d| d.explanation.clone())
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Every value that fed one correction, each carrying its own layer.
    ///
    /// 9.3 is explicit that one layer for the whole record will not do. "The
    /// dose came from the vendor default, the temperature from the user's
    /// learned preference" is the level of detail it requires, so attribution
    /// is per value.
    ///
    /// Three kinds of thing enter a correction and each is recorded as what it
    /// is: what the machine saw, what its knowledge said to aim at, and what
    /// the manufacturer declared the mechanism would do. Collapsing them would
    /// lose the difference between a measurement and an instruction, which is
    /// the first thing anyone reading this after an incident needs.
    fn provenance(
        &self,
        correction: &adi_core::Correction,
        grounds: &Grounds<'_>,
    ) -> Vec<adi_core::ValueProvenance> {
        use adi_core::{ValueProvenance, ValueSource};
        use adi_protocol::knowledge::Layer;

        let variable = &correction.variable;
        let mut inputs = Vec::new();

        // What was measured. Recorded only where the profile says which signal
        // the variable came from and the cycle carries a value: a measurement
        // named in the record and never taken is worse than one left out.
        if let (Some(cycle), Some(signal)) = (grounds.cycle, self.domain.signal_for(variable)) {
            if let Some(observed) = cycle.value(variable) {
                inputs.push(ValueProvenance {
                    variable: variable.clone(),
                    value: observed,
                    // A measurement comes from the machine rather than from a
                    // knowledge layer. Base is the honest answer: no extension
                    // changes what a sensor read.
                    layer: Layer::Base,
                    source: ValueSource::Measured { signal },
                });
            }
        }

        // What the knowledge said to aim at, and which layer said it. This is
        // the entry requirement 5 exists for.
        if let Some(resolution) = grounds.resolution {
            if let Some(target) = resolution.targets.get(variable.0.as_str()) {
                let delta = grounds
                    .diagnosis_for(variable)
                    .and_then(|d| d.desired_delta)
                    .unwrap_or(0.0);

                // The aim point: where the engine was trying to put the
                // variable, rather than the bound it compared against. A range
                // has two ends, and recording either alone would imply the
                // profile targeted it.
                if let Some(observed) = grounds.cycle.and_then(|c| c.value(variable)) {
                    inputs.push(ValueProvenance {
                        variable: variable.clone(),
                        value: Quantity::new(observed.value + delta, observed.unit),
                        layer: target.layer,
                        source: ValueSource::Knowledge {
                            key: variable.0.clone(),
                        },
                    });
                }
            }
        }

        // A requested value came from the person rather than from knowledge,
        // and the difference matters: anyone asking why the machine did that is
        // owed "because it was asked to" where that is the answer.
        if grounds.operator_asked {
            inputs.push(ValueProvenance {
                variable: variable.clone(),
                value: correction.absolute,
                layer: Layer::Base,
                source: ValueSource::Operator,
            });
        }

        // What the manufacturer declared this mechanism does: the response that
        // sized the correction. Without it the arithmetic cannot be replayed,
        // because the step taken is the desired change divided by this.
        if let Some(response) = self
            .skill
            .and_then(|s| s.response(&correction.tool, variable))
        {
            inputs.push(ValueProvenance {
                variable: variable.clone(),
                value: Quantity::new(response.per_unit, correction.delta.unit),
                layer: Layer::Base,
                source: ValueSource::SkillResponse {
                    tool: correction.tool.clone(),
                    parameter: correction.parameter.clone(),
                },
            });
        }

        inputs
    }

    /// What the machine produced, kept so a later request phrased against it
    /// has something to resolve against.
    ///
    /// The settings are what is stored rather than the measurements, because
    /// "the same as last time" means the same settings. What came out is the
    /// result of those, and asking for a result directly is a target, not a
    /// memory.
    fn remember(&mut self, cycle: &Cycle, style: &str) {
        self.memory.push(Remembered {
            cycle_id: cycle.cycle_id.clone(),
            style: Some(style.to_string()),
            settings: cycle.settings.clone(),
            feedback: cycle.feedback.clone(),
        });
    }

    /// What the engine knows the machine is set to, for a request that has no
    /// cycle of its own to read.
    ///
    /// Three sources, weakest first, so the best knowledge available stands.
    ///
    /// The cycle is the base: it records what the machine was set to while it
    /// ran, which is true of that run and may have been superseded since.
    /// `believed` is laid over it, carrying both what the engine has sent and
    /// what the appliance has reported, in the order those happened.
    ///
    /// An appliance that reports its setpoints makes this exact, including
    /// after someone turns a knob by hand, which ADI-P 9.1 requires be
    /// possible at any moment. One that does not leaves the engine with what
    /// it sent and what it last saw, which is the most that can be known about
    /// a machine that does not say.
    ///
    /// Empty is a truthful answer on a session that has observed nothing, sent
    /// nothing and been told nothing. The planner then refuses rather than
    /// assuming a position, and names the parameter it could not place.
    fn present_settings(&self) -> BTreeMap<String, Quantity> {
        let mut present = match self.memory.last() {
            Some(remembered) => self.current_settings(&remembered.settings),
            None => BTreeMap::new(),
        };

        for (parameter, value) in &self.believed {
            present.insert(parameter.clone(), *value);
        }

        present
    }

    /// What the machine is set to now, for the planner to move from.
    ///
    /// Keyed by parameter name, because that is what the planner asks for and
    /// what a cycle records: a setting shares its name with the parameter that
    /// sets it.
    ///
    /// Not via the skill. A skill response names the variable a parameter
    /// *moves*, which is a different question in a different unit: position is
    /// in steps and moves cycle_time, which is in seconds. Reading the
    /// response and handing back the variable's value gives the planner
    /// seconds where it asked for steps, and it correctly refuses. The
    /// conformance runner had this same confusion once.
    fn current_settings(
        &self,
        settings: &BTreeMap<adi_protocol::ids::VariableId, Quantity>,
    ) -> BTreeMap<String, Quantity> {
        let mut current = BTreeMap::new();

        for (variable, value) in settings {
            // Under its own name first, which covers the case where a cycle
            // records a setting by the parameter that sets it.
            current.insert(variable.0.clone(), *value);

            // And under any parameter name the variable ends with. A cycle
            // naturally records `grind_setting` while the tool that moves it
            // declares a parameter called `setting`, and a planner asked for
            // `setting` would otherwise find nothing and refuse a correction
            // it could have made. The conformance runner resolves it the same
            // way; this exists so the two agree.
            for tool in &self.declaration.tools {
                for parameter in tool.parameters.keys() {
                    if variable.0 != *parameter && variable.0.ends_with(parameter.as_str()) {
                        current.entry(parameter.clone()).or_insert(*value);
                    }
                }
            }
        }

        current
    }
}

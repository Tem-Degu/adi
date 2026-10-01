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

    transport: Box<dyn Transport>,

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
        let _ = self.transport.recv(self.heartbeat_interval_ms.min(50) as u32)?;
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

        let current = self.current_settings(&cycle);
        let planner = Planner::new(self.declaration, self.skill, self.domain);
        let context = SelectionContext {
            history: &self.history,
            cycle: Some(&cycle),
            memory: Some(&self.memory),
            rejected: None,
        };

        let corrections = decide(
            &planner,
            self.selector.as_ref(),
            std::slice::from_ref(&diagnosis),
            &context,
            &current,
        )?;

        self.perform(corrections, &cycle.cycle_id)
    }

    /// Serve a request. The path a spoken instruction takes once it has become
    /// structured intent.
    pub fn request(&mut self, intent: &Intent, now_ms: u64) -> Result<Turn, SessionError> {
        self.advance_to(now_ms);
        let resolved = adi_core::request::resolve(intent, &self.memory)?;
        let knowledge = self.domain.knowledge();

        let mut goals = Vec::new();
        for (variable, target) in &resolved.targets {
            goals.push(adi_core::request::as_diagnosis(
                variable,
                *target,
                self.memory.last().and_then(|r| r.setting(variable)),
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
        };

        let corrections = decide(
            &planner,
            self.selector.as_ref(),
            &goals,
            &context,
            &BTreeMap::new(),
        )?;

        self.perform(corrections, "request")
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

        let records = self.record(&corrections, &invocations, cycle_id);
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
                Some(_) => continue,
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

    fn record(
        &self,
        corrections: &[adi_core::Correction],
        invocations: &[Invocation],
        cycle_id: &str,
    ) -> Vec<DecisionRecord> {
        corrections
            .iter()
            .zip(invocations)
            .enumerate()
            .map(|(n, (correction, invocation))| DecisionRecord {
                cycle_id: cycle_id.to_string(),
                decision_id: format!("{cycle_id}/{n}"),
                autonomy_level: self.level.as_str(),
                cause: correction.variable.0.clone(),
                rule_id: String::new(),
                selected_tool: Some(correction.tool.clone()),
                selected_by: self.level.selected_by(),
                inputs: Vec::new(),
                arguments: invocation.numeric_arguments.clone(),
                clamped_from: BTreeMap::new(),
                explanation: String::new(),
            })
            .collect()
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
    fn current_settings(&self, cycle: &Cycle) -> BTreeMap<String, Quantity> {
        let mut current = BTreeMap::new();

        for (variable, value) in &cycle.settings {
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

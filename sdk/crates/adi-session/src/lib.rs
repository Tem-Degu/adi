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

    /// The recipe being served, for the record to name.
    ///
    /// Separate from `resolution` on purpose. That field attributes each value
    /// to the knowledge layer it came from, which is why the request path
    /// leaves it `None` and sets `operator_asked` beside it: a figure a person
    /// named did not come from the manufacturer and must not be recorded as
    /// though it had. Filling it in merely to reach the style would quietly
    /// change attribution in every record on that path.
    ///
    /// This field claims nothing about where any value came from. It says only
    /// what was being made.
    style: Option<&'a str>,
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
    /// The appliance started the operation and then stopped it.
    ///
    /// Distinct from a refusal, which never started. ADI-P 9.1 requires that
    /// an operator can take control by physical means at any moment and that
    /// doing so aborts an engine-initiated operation, so this is the ordinary
    /// outcome of somebody opening a door, and what it means for the machine
    /// is not what a refusal means: part of the operation happened.
    #[error("the appliance stopped the operation: {0:?}")]
    Aborted(RejectReason),
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
    /// An action index that names nothing.
    ///
    /// The list is derived from the declaration for each request, so an index
    /// outside it means the caller is answering against a different list than
    /// the engine offered - a stale one, or another machine's. Refused rather
    /// than clamped into range, because the nearest valid action is a
    /// different physical act and performing it would be a guess.
    #[error("there is no declared action {index}; this machine offers {available}")]
    NoSuchAction { index: usize, available: usize },
}

/// One step of a plan, and the reason `perform` takes a list of these rather
/// than a list of corrections.
///
/// A correction moves a variable towards a number and carries everything that
/// implies: a parameter, a magnitude, a confidence, a diagnosis behind it. An
/// action carries none of that. It is a declared tool a person named, with no
/// arguments at all, and the record of one has to say so rather than invent a
/// variable it moved.
///
/// Both go through the same `perform`, which is deliberate. Two routes to the
/// appliance would mean two places the envelope is checked, and the one used
/// less often is the one that would eventually stop being checked at all.
enum Step {
    Correct(adi_core::Correction),
    Act {
        tool: adi_protocol::ids::SemanticId,
        /// For the explanation. The engine has the identifier; a person
        /// reading the record wants the word they said.
        name: String,
    },
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

/// How long one pass of a wait loop blocks for.
///
/// Short enough that a loop comes up for air about twenty times a second, long
/// enough not to be a busy spin. Unrelated to `control_timeout_ms`, which is
/// how long the APPLIANCE tolerates silence; this is how long the ENGINE sits
/// still before looking up.
const WAIT_SLICE_MS: u32 = 50;

/// An operation the appliance has accepted and is still working on.
///
/// Handed back by [`Session::begin`] so a caller can get on with something
/// else and ask later whether it has finished. Carries the invocation id
/// because the engine must be able to tell this operation's reply from
/// another's: stopping a drink puts two operations in flight at once, and
/// before this existed the loop read whichever reply arrived first as the
/// answer to whatever it happened to be waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    invocation_id: u32,
    tool: adi_protocol::ids::SemanticId,
}

impl Job {
    /// Which tool is running.
    pub fn tool(&self) -> &adi_protocol::ids::SemanticId {
        &self.tool
    }
}

/// How an operation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// It finished.
    Completed,
    /// It was cut short - by a stop, or by the appliance itself.
    Aborted(Option<RejectReason>),
    /// The appliance refused it after having accepted it.
    Refused(RejectReason),
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

    /// What the appliance last said about itself in a heartbeat.
    ///
    /// None until it has said anything. Its heartbeat carries `busy` and
    /// `faulted` and both were being thrown away here, which is how the
    /// engine came to have no idea whether a machine was ready for the next
    /// thing - it knew only that the last operation had reported finishing,
    /// which is not the same moment.
    appliance_busy: Option<bool>,
    appliance_faulted: Option<bool>,

    /// The operation in flight, if any, and how it ended once it has.
    ///
    /// `Some(job, None)` is running; `Some(job, Some(ended))` has finished and
    /// nobody has collected the outcome yet. Held on the session rather than
    /// returned, because the reply arrives through `tick` - which is called
    /// from the caller's own loop, long after the call that started it
    /// returned.
    outstanding: Option<(Job, Option<Ended>)>,
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
            appliance_busy: None,
            appliance_faulted: None,
            outstanding: None,
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

            if self.state_is_in_hand() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Whether the engine knows enough about the machine to take an
    /// instruction about it.
    ///
    /// Both halves, where the machine offers both. Waiting only for the
    /// setpoints returned as soon as the first report arrived, and a reading
    /// published half a second later missed the first request: a relation
    /// stated against the present state was then refused for want of a
    /// reading the appliance was about to send. A machine that declares no
    /// signal measuring an outcome is complete without one.
    fn state_is_in_hand(&self) -> bool {
        let reports_an_outcome = self
            .declaration
            .signals
            .iter()
            .any(|signal| signal.measures.is_some());

        !self.believed.is_empty() && (!reports_an_outcome || !self.measured.is_empty())
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
            // The engine holds no conversation; whatever is talking to the
            // person does, and passes it in on the paths that have one.
            recent: &[],
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

        let turn = self.perform(
            corrections.into_iter().map(Step::Correct).collect(),
            &cycle.cycle_id,
            &Grounds {
                diagnoses: std::slice::from_ref(&diagnosis),
                cycle: Some(&cycle),
                resolution: Some(&resolution),
                operator_asked: false,
                style: Some(resolution.style.as_str()),
            },
        )?;

        // Waited out here because `perform` used to do it. A cycle's
        // corrections are quick, but observing has always returned with
        // the machine settled and nothing should change that quietly.
        self.settle(now_ms)?;
        Ok(turn)
    }

    /// The actions an operator may name on this machine, described.
    ///
    /// The list `act` resolves an index against, exposed so that whatever is
    /// talking to the person can offer the same one. A caller deriving its own
    /// would eventually derive a different order, and an index resolved
    /// against the wrong list performs the wrong physical act.
    ///
    /// It also carries each tool's declared hazards, because confirming with a
    /// person belongs to the layer that can ask them and not to this one.
    pub fn named_actions(&self) -> Vec<adi_core::NamedAction<'_>> {
        adi_core::named_actions(self.declaration, self.domain)
    }

    /// Carry out one of the actions an operator may name. ADI-P 8.2.
    ///
    /// The other half of what a machine does. [`Session::request`] serves a
    /// goal, which is the right shape for every mechanism that moves a
    /// quantity and no shape at all for one that does not: stopping, declaring
    /// a fresh cup, running a rinse. Those are declared tools with no
    /// parameters, so no skill response can describe them and the planner can
    /// never reach one. 8.2 says a declared tool is invocable "when the
    /// operator names it", and this is how.
    ///
    /// `index` is a position in [`Declaration::operator_actions`], which the
    /// engine derives from the declaration and offers with each request. It is
    /// never a tool identifier: 9.3 requirement 3 forbids a model naming a
    /// tool by string, and an index resolved here and stored nowhere obeys it
    /// the same way selection at D2 does.
    ///
    /// Available at every autonomy level, because the decision is not the
    /// engine's or a model's to make. D0 forbids a model producing an
    /// actuation parameter and this produces none; what it performs is the
    /// manufacturer's own declaration, at a person's request.
    ///
    /// A hazard the declaration flags is not checked here. Confirming belongs
    /// to whatever is talking to the person, which is the only layer that can
    /// ask them; `operator_actions` carries the flag so it can.
    /// Carry out a named action, waiting for the machine to finish.
    pub fn act(&mut self, index: usize, now_ms: u64) -> Result<Turn, SessionError> {
        let turn = self.begin_act(index, now_ms)?;
        self.settle(now_ms)?;
        Ok(turn)
    }

    /// Start a named action and come back while it is still running.
    ///
    /// [`Session::act`] waits for it; this does not, so a caller can read the
    /// keyboard, answer a question or send a stop while a drink pours. Wait
    /// for it with [`Session::settle`], or drive `tick` and
    /// [`Session::collect`] in a loop of your own.
    pub fn begin_act(&mut self, index: usize, now_ms: u64) -> Result<Turn, SessionError> {
        self.advance_to(now_ms);

        // Resolved immediately and into owned values. The index means nothing
        // except against the list it came from, and the list is rebuilt from
        // the declaration each time, so nothing may keep one.
        let (tool, name) = {
            // The same list the caller was offered, for the same reason.
            let actions = self.named_actions();
            let action = actions.get(index).ok_or(SessionError::NoSuchAction {
                index,
                available: actions.len(),
            })?;
            (action.tool.clone(), action.name.to_string())
        };

        self.perform(
            vec![Step::Act { tool, name }],
            "action",
            &Grounds {
                // Nothing was diagnosed and no knowledge was resolved. A
                // person asked.
                diagnoses: &[],
                cycle: None,
                resolution: None,
                operator_asked: true,
                style: None,
            },
        )
    }

    /// Serve a request. The path a spoken instruction takes once it has become
    /// structured intent.
    /// Serve a request, waiting for the machine to finish.
    pub fn request(&mut self, intent: &Intent, now_ms: u64) -> Result<Turn, SessionError> {
        let turn = self.begin(intent, now_ms)?;
        self.settle(now_ms)?;
        Ok(turn)
    }

    /// Serve a request and come back while the machine is still working.
    ///
    /// [`Session::request`] waits for it; this does not.
    ///
    /// WHY IT EXISTS. `request` does not return until the appliance reports
    /// the operation COMPLETE, which for a coffee is most of a minute, and the
    /// caller is inside that call for the whole time. So nothing reads the
    /// keyboard or the microphone: during a drink the machine is not slow, it
    /// is deaf. "Stop" cannot be said, "how long?" cannot be answered although
    /// the reading is arriving throughout, and "no, not that one" cannot
    /// interrupt - which is the mechanism by which a machine is supposed to
    /// say what it is about to do and be corrected.
    ///
    /// Everything that happens after the send - the envelope commit, the
    /// record, the Turn - is decided by what was asked for rather than by how
    /// it turned out, so the Turn handed back here is complete and final. Only
    /// the outcome is still to come.
    pub fn begin(&mut self, intent: &Intent, now_ms: u64) -> Result<Turn, SessionError> {
        self.advance_to(now_ms);
        // With the readings, so a relation stated against the present state
        // resolves. "A bit warmer" is relative to what the appliance reports
        // now, which no past cycle can supply and a machine on its first day
        // has none of.
        let resolved =
            adi_core::request::resolve_with(intent, &self.memory, &self.measured)?;
        let knowledge = self.domain.knowledge();
        let planner = Planner::new(self.declaration, self.skill, self.domain);

        // A drink asked for by name carries its own definition, and this path
        // did not read it.
        //
        // The knowledge layers say what a style is: so many grams in the cup,
        // at such a temperature. `observe` resolves that when judging a
        // finished cycle. `request` did not, so an intent naming a style and
        // no target produced no goals at all and returned below as "nothing
        // needed doing" - a valid order, silently ignored. Whoever wrote the
        // interpreter then had to invent the amount, which put the recipe in
        // two places and made it a guess in one of them.
        //
        // Only for `produce`. Asking for a drink means make one to its
        // definition; `adjust` means change the one thing named, and seeding
        // the rest of a style there would move what nobody asked for.
        let mut targets = resolved.targets.clone();

        // Which of these the person actually named. The rest arrive from the
        // style below, and the record has to be able to tell them apart or it
        // reports a recipe's figure as something somebody asked for.
        let stated: std::collections::BTreeSet<adi_protocol::ids::VariableId> =
            targets.keys().cloned().collect();
        if intent.kind == adi_protocol::intent::IntentKind::Produce {
            if let Some(style) = resolved.style.as_deref() {
                let resolution = knowledge
                    .resolve(style, resolved.material.as_deref())
                    .map_err(|e| SessionError::Knowledge(e.to_string()))?;

                // `style_targets`, not `targets`. The latter folds in the
                // profile's quality constraints, which say where a variable
                // may not go rather than where it should be aimed; seeding
                // from those had every coffee order trying to drive the brew
                // temperature to the middle of its legal range.
                for (name, target) in resolution.style_targets.iter() {
                    let variable = adi_protocol::ids::VariableId(name.clone());

                    // Only what this machine can move. A profile models
                    // outcomes a given machine may have no mechanism for, and
                    // seeding one of those turns a request the machine could
                    // have served into a refusal that helps nobody. What the
                    // person stated themselves is still held to strictly,
                    // because `or_insert_with` never overwrites.
                    if planner.candidates(&variable).is_empty() {
                        continue;
                    }

                    // A style states a band rather than a point, so aim at the
                    // middle of it. Either end would be a target the cycle
                    // path then reads as borderline.
                    let band = &target.value;
                    targets.entry(variable).or_insert_with(|| {
                        Quantity::new((band.min + band.max) / 2.0, band.unit)
                    });
                }
            }
        }

        let mut goals = Vec::new();
        for (variable, target) in &targets {
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
                if stated.contains(variable) {
                    adi_core::request::Asked::Stated
                } else {
                    adi_core::request::Asked::Recipe
                },
            ));
        }

        if goals.is_empty() {
            return Ok(Turn {
                nominal: true,
                ..Default::default()
            });
        }

        let context = SelectionContext {
            // The engine holds no conversation; whatever is talking to the
            // person does, and passes it in on the paths that have one.
            recent: &[],
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
            corrections.into_iter().map(Step::Correct).collect(),
            "request",
            &Grounds {
                diagnoses: &goals,
                // A request is served without observing a cycle, so there is
                // nothing measured to attribute. The engine knows what it was
                // asked for and not what the machine last did.
                cycle: None,
                resolution: None,
                operator_asked: true,
                // Known since the top of this function and, until now,
                // discarded right here. An `Adjust` names no style and so
                // records none, which is correct: changing the syrup is not
                // making a drink.
                style: resolved.style.as_deref(),
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

            // What the appliance says about itself, which was discarded.
            //
            // Its heartbeat answers ours and carries both of these. Knowing
            // whether it is busy is what tells the engine it may send the next
            // step of a plan: an operation reporting COMPLETE and the machine
            // being ready for another are two different moments, and on a Jura
            // the gap between them is long enough that the next step was
            // refused for a precondition that had not caught up yet.
            Message::Heartbeat { busy, faulted } => {
                if busy.is_some() {
                    self.appliance_busy = *busy;
                }
                if faulted.is_some() {
                    self.appliance_faulted = *faulted;
                }
            }

            // The reply to an operation that was started and left running.
            //
            // This arm did not exist. `tick` drains everything the appliance
            // has to say and hands each message here, so a COMPLETED arriving
            // during a tick was dropped on the floor - harmless only because
            // nothing was ever left running across a tick. The moment an
            // operation is started and waited on from the caller's own loop,
            // this is the message that says it is over, and without this arm
            // it would never be noticed.
            //
            // Matched by id. A stop puts two operations in flight, and the
            // drink's ABORTED arrives before the stop's COMPLETED; reading the
            // first as the answer to the second is how a machine comes to
            // report that it could not stop something it had just stopped.
            Message::Response { id, outcome, reason } => {
                let Some((job, ended)) = self.outstanding.as_mut() else {
                    return;
                };
                if job.invocation_id != *id || ended.is_some() {
                    return;
                }
                *ended = match outcome {
                    Outcome::Completed => Some(Ended::Completed),
                    Outcome::Aborted => Some(Ended::Aborted(*reason)),
                    Outcome::Rejected => {
                        Some(Ended::Refused(reason.unwrap_or(RejectReason::Fault)))
                    }
                    // Still working. An appliance may report RUNNING more than
                    // once over a long operation.
                    Outcome::Accepted | Outcome::Running => None,
                };
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
        steps: Vec<Step>,
        cycle_id: &str,
        grounds: &Grounds<'_>,
    ) -> Result<Turn, SessionError> {
        let mut invocations = Vec::new();
        for step in &steps {
            let id = self.next_invocation_id;
            self.next_invocation_id = self.next_invocation_id.wrapping_add(1).max(1);

            invocations.push(match step {
                Step::Correct(correction) => {
                    Invocation::new(correction.tool.clone(), id)
                        .with_number(&correction.parameter, correction.absolute)
                }
                // No arguments, and that is the whole safety of this path:
                // every value the appliance uses is one it declared itself.
                // `operator_actions` admits a tool only where every parameter
                // is optional, so there is nothing missing here for the
                // appliance to refuse.
                Step::Act { tool, .. } => Invocation::new(tool.clone(), id),
            });
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
        for (nth, invocation) in invocations.iter().enumerate() {
            // Wait for the step before it, and only for that.
            //
            // A plan's steps are sequential: an appliance doing one thing
            // refuses the next with BUSY, so firing them back to back gets
            // everything after the first refused. Found by asking for a coffee
            // with syrup in it - two goals, two tools - once
            // `send_and_confirm` began returning as soon as the appliance
            // accepted rather than when it had finished. The syrup was
            // accepted and the brew refused half a millisecond later. No test
            // caught it: every fake appliance in the suite answers COMPLETED
            // instantly and so is never busy.
            //
            // NOT before the first step, which is the whole difference between
            // the two cases. A plan arriving while something else is running
            // is an interruption - "stop that", or a different drink instead -
            // and must go out while that something is still going, or the
            // appliance has nothing left to interrupt. Whether it is allowed
            // to interrupt is the appliance's call: libadip honours
            // `abortable` on what is running and refuses BUSY otherwise.
            //
            // Waiting here unconditionally made a stop wait for the drink it
            // was meant to cut short, which the stop's own test caught.
            //
            // The LAST step is deliberately left running: that is what lets a
            // caller get on with something while a drink pours.
            if nth > 0 {
                if self.working() {
                    self.settle(self.now_ms)?;
                }
                // And then until the appliance says it is free, which is a
                // later moment than the operation ending.
                self.wait_until_free(self.now_ms)?;

                // And then, if it still says no, ask again for a moment.
                //
                // WHY A RETRY AND NOT A LONGER WAIT. An appliance's view of
                // itself can lag its own operations, and the engine cannot see
                // far enough in to know when it has caught up. The Jura
                // reports a product finished, libadip clears `busy` on the
                // strength of it, and `state == idle` - which the firmware
                // computes from the machine's own status poll - becomes true
                // some time later. Measured: a milk dispense followed by a
                // brew was refused PRECONDITION_FAILED, while every signal
                // read correctly a few seconds afterwards. Nothing was wrong
                // except the order of two clocks.
                //
                // So the engine asks again rather than guessing how long to
                // wait, and gives up quickly. Only between the steps of one
                // plan: a first step refused is a refusal a person is owed
                // straight away, and retrying it would be the engine arguing
                // with the appliance about something it was told once.
                self.send_with_patience(invocation)?;
                self.guard.commit(invocation, &mut self.safety);
                performed.push(invocation.clone());
                continue;
            }

            self.send_and_confirm(invocation)?;
            self.guard.commit(invocation, &mut self.safety);
            performed.push(invocation.clone());
        }

        // Provisional, and superseded by the appliance's own report when one
        // arrives. Written here because the invocation was accepted, so this
        // is the best the engine can believe until it is told otherwise.
        for step in &steps {
            // Only a correction moves a setting. An action sends no value, so
            // there is no new belief to hold, and writing one would have the
            // engine believe a parameter had changed because something
            // unrelated ran.
            if let Step::Correct(correction) = step {
                self.believed
                    .insert(correction.parameter.clone(), correction.absolute);
            }
        }

        let records = self.record(&steps, &invocations, cycle_id, grounds);
        self.history.extend(records.iter().cloned());

        Ok(Turn {
            records,
            performed,
            nominal: false,
        })
    }

    /// Send an invocation and wait only until the appliance takes it on.
    ///
    /// It used to wait for the operation to FINISH, which for a coffee is most
    /// of a minute. The caller sat inside this call for the whole time, so
    /// nothing read the keyboard or the microphone: during a drink the machine
    /// was not slow, it was deaf. Everything after the send in `perform` - the
    /// envelope commit, the record, the Turn - is decided by what was asked
    /// for rather than by how it turned out, so returning here as soon as the
    /// appliance has accepted changes none of it.
    ///
    /// What is still running is left in `outstanding`, and the reply that ends
    /// it arrives through `tick` like any other message.
    fn send_and_confirm(&mut self, invocation: &Invocation) -> Result<(), SessionError> {
        self.transport.send(&adi_wire::invocation(invocation)?)?;

        let keepalive =
            std::time::Duration::from_millis((self.control_timeout_ms / 3).max(200));

        // Only long enough for the appliance to answer, not long enough for it
        // to do the work. An appliance that says nothing at all within its own
        // declared tolerance is not listening.
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(self.control_timeout_ms + 1000);
        let mut spoke_at = std::time::Instant::now();

        while std::time::Instant::now() < deadline {
            if spoke_at.elapsed() >= keepalive {
                self.transport.send(&Message::Heartbeat {
                    busy: None,
                    faulted: None,
                })?;
                spoke_at = std::time::Instant::now();
            }

            match self.transport.recv(self.control_timeout_ms as u32)? {
                Some(Message::Response { id, outcome, reason })
                    if id == invocation.invocation_id =>
                {
                    // By id. Anything else is another operation's business,
                    // and reading it as this one's answer is how an engine
                    // comes to report the wrong outcome for the wrong job.
                    return match outcome {
                        Outcome::Rejected => {
                            Err(SessionError::Refused(reason.unwrap_or(RejectReason::Fault)))
                        }
                        Outcome::Aborted => {
                            Err(SessionError::Aborted(reason.unwrap_or(RejectReason::Fault)))
                        }
                        // Over before this returned. Nothing is left running.
                        Outcome::Completed => Ok(()),
                        // Taken on. The caller owns the waiting from here.
                        Outcome::Accepted | Outcome::Running => {
                            self.outstanding = Some((
                                Job {
                                    invocation_id: invocation.invocation_id,
                                    tool: invocation.tool.clone(),
                                },
                                None,
                            ));
                            Ok(())
                        }
                    };
                }
                Some(other) => {
                    // Not this question's answer, and not rubbish either: an
                    // appliance that has just acted reports its new setting,
                    // and that report arrives here.
                    self.absorb(&other);
                }
                None => {
                    // An appliance that has not answered an invocation at all
                    // is not thinking about it, and waiting out the whole
                    // tolerance to say so helps nobody. Reported at once,
                    // which is also what `silence_from_the_appliance_is_an_
                    // error_not_a_success` asserts - losing this turned that
                    // test from instant into a three second spin, which is
                    // how it was noticed.
                    break;
                }
            }
        }

        Err(SessionError::Silent)
    }

    /// Whether an operation is still running.
    pub fn working(&self) -> bool {
        matches!(self.outstanding, Some((_, None)))
    }

    /// The operation in flight, if there is one.
    pub fn job(&self) -> Option<&Job> {
        self.outstanding.as_ref().map(|(job, _)| job)
    }

    /// Collect the outcome of an operation that has ended.
    ///
    /// None while it is still running, and None again once collected, so a
    /// loop may call it every time round without having to remember whether it
    /// has already asked.
    pub fn collect(&mut self) -> Option<Ended> {
        let ended = self.outstanding.as_ref().and_then(|(_, e)| e.clone())?;
        self.outstanding = None;
        Some(ended)
    }

    /// Send a step, asking again briefly if the appliance is not ready yet.
    ///
    /// Only for a step after the first in a plan, where the engine knows it
    /// has just finished something and that the appliance's own view of itself
    /// may not have caught up. See the call site for what was measured.
    ///
    /// Anything other than "not ready" is passed straight back. A refusal for
    /// an interlock, an envelope breach or an argument out of range is a
    /// settled answer, and asking again would only delay reporting it.
    fn send_with_patience(&mut self, invocation: &Invocation) -> Result<(), SessionError> {
        // Bounded by the appliance's own `control_timeout_ms`, not by a number
        // chosen here.
        //
        // THIS WAS THREE TRIES OVER 700ms AND IT WAS A COIN FLIP. Bob's
        // firmware refreshes the machine's status - which is what decides
        // `state == idle` - once a second, so a window shorter than that
        // catches a fresh reading only if it happens to land after one. Milk
        // followed by syrup passed and failed on identical input, and the
        // value being asked for looked like the cause for a while because the
        // runs that happened to pass had a different one.
        //
        // `control_timeout_ms` is the right bound because it is the appliance
        // saying how long it may go without hearing from the engine. A machine
        // whose own view of itself has not settled within that has a problem
        // worth reporting, rather than one worth waiting longer for.
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(self.control_timeout_ms.max(1_000));
        let mut last;

        loop {
            match self.send_and_confirm(invocation) {
                Ok(()) => return Ok(()),
                Err(SessionError::Refused(reason))
                    if matches!(
                        reason,
                        RejectReason::Busy | RejectReason::PreconditionFailed
                    ) =>
                {
                    last = SessionError::Refused(reason);
                }
                // Anything else is a settled answer. An interlock, an envelope
                // breach or an argument out of range will not become true by
                // being asked again.
                Err(other) => return Err(other),
            }

            if std::time::Instant::now() >= deadline {
                return Err(last);
            }

            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// Whether the appliance last said it was faulted.
    pub fn appliance_faulted(&self) -> bool {
        self.appliance_faulted == Some(true)
    }

    /// Wait until the appliance says it is free, having finished something.
    ///
    /// WHY THIS IS NOT THE SAME AS WAITING FOR THE OPERATION TO END. An
    /// operation reporting COMPLETE means the mechanism has stopped. It does
    /// not mean the appliance is ready for the next instruction, and the two
    /// moments are not the same: a Jura product finishes, and `state == idle`
    /// becomes true only when the firmware next refreshes what it publishes.
    /// The engine used to send the next step of a plan into that gap and have
    /// it refused for a precondition that had not caught up.
    ///
    /// Found by asking for milk and syrup in one drink. Each worked alone;
    /// milk followed by anything was refused. Syrup followed by anything was
    /// fine, because our own pump finishes and becomes ready in the same
    /// breath - only the Jura has the gap, which is why nothing had met it.
    ///
    /// An appliance that does not report `busy` at all leaves this returning
    /// at once, which is the old behaviour and no worse than it was.
    fn wait_until_free(&mut self, now_ms: u64) -> Result<(), SessionError> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(self.control_timeout_ms + 1000);

        while std::time::Instant::now() < deadline {
            // Asked rather than waited for. `tick` heartbeats only when one is
            // due, so a tight loop over it would sit here reading nothing; the
            // appliance answers a heartbeat with a heartbeat, so the way to
            // get a fresh answer is to ask.
            self.transport.send(&Message::Heartbeat {
                busy: None,
                faulted: None,
            })?;

            for _ in 0..64 {
                match self.transport.recv(WAIT_SLICE_MS)? {
                    Some(message) => self.absorb(&message),
                    None => break,
                }
            }

            if self.appliance_busy != Some(true) {
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_millis(WAIT_SLICE_MS as u64));
        }

        // Still busy after its own declared tolerance. Not an error here: the
        // next step is sent and the appliance refuses it if it must, which is
        // a refusal with a reason rather than one invented by the engine.
        let _ = now_ms;
        Ok(())
    }

    /// Wait for the operation in flight to end, driving ticks.
    ///
    /// What `request` and `act` do for a caller with nothing else to be
    /// getting on with, and what a caller that DOES have something else to do
    /// writes for itself: tick, look at the outcome, and in between read the
    /// keyboard, answer a question, or send a stop. The ticking is the part
    /// that must not stop - it carries the heartbeat, and an appliance that
    /// hears nothing for its `control_timeout_ms` concludes its controller has
    /// died and abandons what it was doing.
    pub fn settle(&mut self, now_ms: u64) -> Result<Option<Ended>, SessionError> {
        let Some((job, _)) = self.outstanding.clone() else {
            return Ok(None);
        };

        let promised = self
            .declaration
            .tools
            .iter()
            .find(|tool| tool.id == job.tool)
            .and_then(|tool| tool.duration_ms_max)
            .unwrap_or(0);
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(promised + self.control_timeout_ms + 1000);

        let mut at_ms = now_ms;
        while std::time::Instant::now() < deadline {
            self.tick(at_ms)?;
            if let Some(ended) = self.collect() {
                return Ok(Some(ended));
            }

            // Paced here rather than left to the transport. A serial port
            // blocks for its timeout and paces this loop by itself, but an
            // in-memory transport answers instantly, and a loop that trusted
            // the transport to slow it down would spin a core flat for the
            // length of a coffee. That is a real cost on a Pi that is also
            // running the model.
            if !self.answered_last_tick {
                std::thread::sleep(std::time::Duration::from_millis(
                    WAIT_SLICE_MS as u64,
                ));
            }

            at_ms = at_ms.saturating_add(self.heartbeat_interval_ms);
        }

        // Nothing said within the operation's own declared maximum. Left
        // outstanding rather than cleared: the engine does not know that it
        // stopped, and saying so would be worse than saying nothing.
        Ok(None)
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
        steps: &[Step],
        invocations: &[Invocation],
        cycle_id: &str,
        grounds: &Grounds<'_>,
    ) -> Vec<DecisionRecord> {
        steps
            .iter()
            .zip(invocations)
            .enumerate()
            .map(|(n, (step, invocation))| {
                let correction = match step {
                    Step::Correct(correction) => correction,
                    Step::Act { tool, name } => {
                        return Self::action_record(
                            self.level.as_str(),
                            cycle_id,
                            n,
                            tool,
                            name,
                            invocation,
                        )
                    }
                };
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
                    // What this was making, where it was making something.
                    // `selected_tool` says which mechanism moved, which on a
                    // machine whose one brewer serves eight recipes does not
                    // say which drink.
                    style: grounds.style.map(str::to_string),
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

    /// The record of an action a person named. ADI-P 8.2, 9.3 requirements 4
    /// and 5.
    ///
    /// Thin compared with a correction's, and every empty field is empty for a
    /// reason rather than for want of plumbing.
    ///
    /// `inputs` is empty because no value fed this. A correction is the output
    /// of arithmetic over a reading, a target and a declared response, and 9.3
    /// requires each of those to be attributed. Here a person said a word and
    /// the engine looked up which declared tool it named. Listing something
    /// under inputs would imply a calculation that did not happen.
    ///
    /// `rule_id` is empty for the same reason: no profile rule fired. What
    /// produced this decision is in `selected_by`, and the honest value there
    /// is `Operator` - neither the engine working it out nor a model choosing
    /// freely.
    ///
    /// `arguments` comes from the invocation and is therefore empty too, which
    /// is the property that makes this path safe rather than an omission.
    ///
    /// Taken as an associated function rather than a method because `record`
    /// holds a borrow of `self` while matching, and this needs none of it.
    fn action_record(
        level: &'static str,
        cycle_id: &str,
        n: usize,
        tool: &adi_protocol::ids::SemanticId,
        name: &str,
        invocation: &Invocation,
    ) -> DecisionRecord {
        DecisionRecord {
            cycle_id: cycle_id.to_string(),
            decision_id: format!("{cycle_id}/{n}"),
            autonomy_level: level,
            // Not a diagnosis from the profile, and it must not read like one.
            // A profile cause names something the machine observed to be wrong;
            // nothing was wrong here and nothing was diagnosed.
            cause: "operator_named_action".to_string(),
            rule_id: String::new(),
            selected_tool: Some(tool.clone()),
            selected_by: adi_core::SelectedBy::Operator,
            inputs: Vec::new(),
            arguments: invocation.numeric_arguments.clone(),
            clamped_from: BTreeMap::new(),
            // No recipe: a named action IS its own identity. The tool
            // `x.bob.drink.cappuccino` names the drink, which is why "another
            // one" worked for the machine's own products while a style-based
            // drink left a model nothing to go on.
            style: None,
            explanation: format!("The operator asked for {name}."),
        }
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

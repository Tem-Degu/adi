//! Autonomy levels.
//!
//! The model is present at every level. It turns what a person said into
//! structured intent before any of this runs, and it explains what happened
//! afterwards. Those jobs are not graded here, because they touch no actuation
//! value and carry no risk that needs grading.
//!
//! What these levels grade is what happens *after* intent: how much of the
//! action itself the model decides.
//!
//! | Level | Model decides | Engine decides |
//! |-------|---------------|----------------|
//! | D0    | intent only | which tools, and every value |
//! | D1    | intent, and which tool | every value |
//! | D2    | intent, which tools, and their values | validates, clamps, may refuse |
//!
//! [`AutonomyLevel::default`] is D2. A machine that can only do what someone
//! anticipated is a program with a microphone attached, and the point of the
//! Safe Action Protocol is that the model does not need to be trusted for the
//! machine to be safe: every value is clamped here, and the appliance validates
//! independently afterwards whatever the engine sends.
//!
//! D0 remains the level to pin where a decision must replay identically, which
//! is why the conformance corpus runs at D0.
//!
//! D2 refuses to initialise against a declaration with no stateful invariants
//! or rate limits. Individually valid actions compose into invalid sequences,
//! and a model choosing its own magnitudes is exactly the case where that
//! matters. The refusal is the point of the level system, not an inconvenience.

use adi_core::domain::Diagnosis;
use adi_core::memory::Memory;
use adi_core::plan::{Candidate, Correction, Planner};
use adi_core::record::{DecisionRecord, SelectedBy};
use adi_core::Cycle;
use adi_protocol::ids::VariableId;
use adi_protocol::units::Quantity;
use adi_protocol::{Declaration, Intent, SemanticId};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "UPPERCASE")]
pub enum AutonomyLevel {
    /// Engine chooses tools and values. Fully reproducible.
    D0,
    /// Model chooses the tool from declared options. Engine computes values.
    D1,
    /// Model chooses tools and values. Requires a stateful envelope.
    D2,
}

/// D2. See the module documentation for why.
impl Default for AutonomyLevel {
    fn default() -> Self {
        AutonomyLevel::D2
    }
}

impl AutonomyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyLevel::D0 => "D0",
            AutonomyLevel::D1 => "D1",
            AutonomyLevel::D2 => "D2",
        }
    }

    pub fn selected_by(self) -> SelectedBy {
        match self {
            AutonomyLevel::D0 => SelectedBy::Engine,
            AutonomyLevel::D1 | AutonomyLevel::D2 => SelectedBy::Model,
        }
    }
}

/// Everything the model is given to reason with.
///
/// The reasoning is the model's job, so this has to hold what reasoning needs.
/// Without it a model sees only the diagnosis and the candidate list, which is
/// exactly what the engine used to rank that list: it could disagree with the
/// ranking but never on evidence.
///
/// `memory` is the same store the engine resolves relative targets against.
/// Both sides see it, which is the point. The model needs it to work out *which*
/// earlier cycle someone means and whether that one was any good; the engine
/// needs it to turn "half of that one" into a number that can be checked later.
/// A model given the request but not the history would be asked to interpret
/// "last time" without being told what last time was.
#[derive(Clone, Copy, Default)]
pub struct SelectionContext<'a> {
    /// Earlier decisions on this machine, most recent first.
    pub history: &'a [DecisionRecord],
    /// The cycle just observed, where there was one. A request that arrives
    /// before anything has run has none.
    pub cycle: Option<&'a Cycle>,
    /// What the machine remembers producing.
    pub memory: Option<&'a dyn Memory>,
    /// Why the previous answer was refused, on a second attempt.
    ///
    /// `None` on the first attempt. A chooser that can pass this to a model
    /// gives it the one thing it needs to correct a recoverable mistake, such
    /// as two steps moving the same variable. Carried here rather than added to
    /// the chooser traits so that a chooser which has no use for it is
    /// unaffected.
    pub rejected: Option<&'a str>,
}

/// Hand-written because `dyn Memory` is not `Debug`, and requiring it would
/// force every appliance-side store to carry a formatter it never uses.
impl std::fmt::Debug for SelectionContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionContext")
            .field("history", &self.history.len())
            .field("cycle", &self.cycle.is_some())
            .field(
                "remembered",
                &self.memory.map(|m| m.recent(usize::MAX).len()),
            )
            .finish()
    }
}

impl<'a> SelectionContext<'a> {
    pub fn new(history: &'a [DecisionRecord], cycle: Option<&'a Cycle>) -> Self {
        Self {
            history,
            cycle,
            memory: None,
            rejected: None,
        }
    }

    /// The same context, carrying why the previous answer was refused.
    pub fn with_rejection(mut self, reason: &'a str) -> Self {
        self.rejected = Some(reason);
        self
    }

    pub fn with_memory(mut self, memory: &'a dyn Memory) -> Self {
        self.memory = Some(memory);
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error(
        "D2 requires a declaration with stateful invariants or rate limits; this one has neither"
    )]
    StatefulEnvelopeRequired,
    #[error("autonomy level {0} needs a model backend; construct the selector directly with one")]
    NoModelBackend(&'static str),
    #[error("no candidate tool can move '{0}'")]
    NoCandidate(String),
    #[error("model selected '{0}', which is not among the declared candidates")]
    UndeclaredSelection(String),
    #[error("model returned a malformed selection: {0}")]
    MalformedSelection(String),
    /// The backend could not be reached or did not answer. Distinct from
    /// answering badly, which is MalformedSelection, and from there being no
    /// backend at all, which is NoModelBackend and a configuration error.
    #[error("the model backend failed: {0}")]
    Backend(String),
    #[error("the plan is empty")]
    EmptyPlan,
    #[error("no goal was given, so there is nothing to plan")]
    NoGoals,
    #[error("two goals both name '{0}'; a plan cannot pursue one variable twice")]
    DuplicateGoal(String),
    #[error("steps '{first}' and '{second}' both move '{variable}' in one iteration")]
    ConflictingSteps {
        first: String,
        second: String,
        variable: String,
    },
    #[error(transparent)]
    Core(#[from] adi_core::CoreError),
}

/// One step of a proposed plan.
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub tool: SemanticId,
    pub parameter: String,
    /// The absolute value for the parameter. Set only at D2, where the model
    /// supplies it. At D0 and D1 the engine computes it.
    pub value: Option<Quantity>,
}

/// What a selector returns.
///
/// A sequence rather than a single action, because a request like "make it
/// weaker and smaller than last time" is several tools with a relationship
/// between them, and a selector that could only return one action could never
/// express it. A correction is simply a plan of length one.
#[derive(Debug, Clone)]
pub struct Selection {
    pub steps: Vec<PlanStep>,
}

impl Selection {
    pub fn one(tool: SemanticId, parameter: String, value: Option<Quantity>) -> Self {
        Self {
            steps: vec![PlanStep {
                tool,
                parameter,
                value,
            }],
        }
    }
}

/// One thing the plan is meant to achieve.
///
/// A correction produces one. A request produces one per variable the person
/// asked about, so "weaker and smaller than last time" is two.
pub type Goal = Diagnosis;

/// The one seam between deterministic and model-driven behaviour.
pub trait ActionSelector {
    fn level(&self) -> AutonomyLevel;

    /// Choose from the candidates the planner derived from declaration and
    /// skill. The candidate list is the safety boundary: a selector can only
    /// ever return something the machine actually declared.
    ///
    /// `goals` may hold more than one, and `candidates` then spans several
    /// variables. Each candidate names the variable it serves, so a selector
    /// can tell which goal a tool addresses.
    fn select(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Selection, DecisionError>;
}

/// Goals must be well formed before anything is offered to a selector.
///
/// Two goals on one variable would be a caller asking for a thing and its
/// opposite in the same breath, and whichever step won would be arbitrary.
fn check_goals(goals: &[Goal]) -> Result<(), DecisionError> {
    if goals.is_empty() {
        return Err(DecisionError::NoGoals);
    }
    let mut seen: Vec<&VariableId> = Vec::with_capacity(goals.len());
    for goal in goals {
        let variable = goal
            .target_variable
            .as_ref()
            .ok_or_else(|| DecisionError::NoCandidate("unspecified".into()))?;
        if seen.contains(&variable) {
            return Err(DecisionError::DuplicateGoal(variable.0.clone()));
        }
        seen.push(variable);
    }
    Ok(())
}

fn first_variable(goals: &[Goal]) -> String {
    goals
        .first()
        .and_then(|g| g.target_variable.as_ref())
        .map(|v| v.0.clone())
        .unwrap_or_else(|| "unspecified".into())
}

/// The most confident candidate serving one goal.
///
/// The list is already sorted by confidence, so the first match is the best
/// one for that variable.
fn best_for<'a>(goal: &Goal, candidates: &'a [Candidate]) -> Result<&'a Candidate, DecisionError> {
    let variable = goal
        .target_variable
        .as_ref()
        .ok_or_else(|| DecisionError::NoCandidate("unspecified".into()))?;
    candidates
        .iter()
        .find(|c| c.variable == *variable)
        .ok_or_else(|| DecisionError::NoCandidate(variable.0.clone()))
}

/// D0. Takes the most confident candidate for each goal.
pub struct EngineSelector;

impl ActionSelector for EngineSelector {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D0
    }

    fn select(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        _context: &SelectionContext<'_>,
    ) -> Result<Selection, DecisionError> {
        check_goals(goals)?;
        let mut steps = Vec::with_capacity(goals.len());
        for goal in goals {
            let chosen = best_for(goal, candidates)?;
            steps.push(PlanStep {
                tool: chosen.tool.clone(),
                parameter: chosen.parameter.clone(),
                value: None,
            });
        }
        Ok(Selection { steps })
    }
}

/// Anything that can answer a closed-schema question. Implemented over a local
/// model in the shipping build, and over a fixture in tests.
pub trait ToolChooser {
    /// Given the goals, the declared candidates and what has happened before,
    /// return the index of the candidate chosen for each goal.
    ///
    /// One index per goal, in the same order. There is no way to abstain: a
    /// goal the person asked for must be served or the whole plan refused,
    /// never quietly dropped.
    fn choose(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Vec<usize>, String>;
}

/// D1. Model picks the tools, engine computes the values.
pub struct ModelToolSelector<C: ToolChooser> {
    pub chooser: C,
}

impl<C: ToolChooser> ActionSelector for ModelToolSelector<C> {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D1
    }

    fn select(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Selection, DecisionError> {
        check_goals(goals)?;
        if candidates.is_empty() {
            return Err(DecisionError::NoCandidate(first_variable(goals)));
        }

        let indices = self
            .chooser
            .choose(goals, candidates, context)
            .map_err(DecisionError::MalformedSelection)?;
        if indices.len() != goals.len() {
            return Err(DecisionError::MalformedSelection(format!(
                "{} goals were given but {} choices came back",
                goals.len(),
                indices.len()
            )));
        }

        let mut steps = Vec::with_capacity(indices.len());
        for index in indices {
            let chosen = candidates
                .get(index)
                .ok_or_else(|| DecisionError::UndeclaredSelection(index.to_string()))?;
            steps.push(PlanStep {
                tool: chosen.tool.clone(),
                parameter: chosen.parameter.clone(),
                value: None,
            });
        }
        Ok(Selection { steps })
    }
}

/// One step as the model proposes it: an index into the candidate list, and a
/// magnitude. The index is checked against the list; the magnitude is checked
/// by the planner and again by the appliance.
#[derive(Debug, Clone, Copy)]
pub struct ProposedStep {
    pub candidate: usize,
    /// Absolute value for the parameter, in the parameter's declared unit.
    pub value: f64,
}

/// Anything that can propose a sequence of actions with their magnitudes.
pub trait ActionChooser {
    fn choose(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Vec<ProposedStep>, String>;
}

/// D2. Model proposes tools and values; the engine verifies the whole plan.
///
/// Construct with [`ModelActionSelector::new`], which refuses a declaration
/// without a stateful envelope.
pub struct ModelActionSelector<C: ActionChooser> {
    chooser: C,
}

impl<C: ActionChooser> ModelActionSelector<C> {
    pub fn new(chooser: C, declaration: &Declaration) -> Result<Self, DecisionError> {
        if !declaration.has_stateful_envelope() {
            return Err(DecisionError::StatefulEnvelopeRequired);
        }
        Ok(Self { chooser })
    }
}

impl<C: ActionChooser> ActionSelector for ModelActionSelector<C> {
    fn level(&self) -> AutonomyLevel {
        AutonomyLevel::D2
    }

    fn select(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Selection, DecisionError> {
        check_goals(goals)?;
        if candidates.is_empty() {
            return Err(DecisionError::NoCandidate(first_variable(goals)));
        }

        let proposed = self
            .chooser
            .choose(goals, candidates, context)
            .map_err(DecisionError::MalformedSelection)?;
        if proposed.is_empty() {
            return Err(DecisionError::EmptyPlan);
        }

        let mut steps = Vec::with_capacity(proposed.len());
        for step in proposed {
            let chosen = candidates
                .get(step.candidate)
                .ok_or_else(|| DecisionError::UndeclaredSelection(step.candidate.to_string()))?;
            steps.push(PlanStep {
                tool: chosen.tool.clone(),
                parameter: chosen.parameter.clone(),
                // Tagged with the parameter's declared unit rather than left
                // bare. An untagged number is refused by the planner, which is
                // how a magnitude with no stated unit used to slip through.
                value: Some(Quantity::new(step.value, chosen.param_unit)),
            });
        }
        Ok(Selection { steps })
    }
}

// ------------------------------------------------------------ the model seam

/// What the model made of what someone said.
///
/// Asking is a first-class outcome, not a failure. The alternative to asking is
/// guessing, and on this path a guess becomes a physical action. A machine that
/// says "the one this morning, or yesterday's?" is doing its job; one that picks
/// silently and pours is not.
///
/// A question is text going to a person. It reaches no actuation value, so the
/// model has the same freedom here it has in understanding, and none of the
/// checks below apply to it.
/// Serialised as `{"intent": {...}}` or `{"clarify": {...}}`.
///
/// Externally tagged rather than internally, because `Intent` already carries a
/// `kind` field and an internal tag would collide with it. This is the exact
/// shape a model is constrained to produce; see `adi-models::schema`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Understood {
    /// The request was understood well enough to act on.
    Intent(Intent),
    /// The model needs the person to say more before anything happens.
    Clarify {
        /// What to ask. Shown to the person as written.
        question: String,
        /// Why it could not be resolved, for the record rather than the person.
        because: String,
    },
    /// The person asked something rather than requesting anything.
    ///
    /// Nothing is actuated. This is the half of a domain intelligence that is
    /// knowing rather than doing, and before it existed a question had to come
    /// back as an intent, which made the machine act, or as a clarification,
    /// which asked a question of someone who had just asked one.
    ///
    /// Unlike an intent, this is **not safe by construction**. An intent is
    /// checked against the declaration, clamped and validated by the appliance,
    /// so a wrong one becomes a refused plan. Text goes straight to a person and
    /// nothing checks it. The mitigation is to answer from supplied knowledge
    /// and to record what was drawn on, which narrows the risk rather than
    /// removing it. ADI-P 9.4.3 states this asymmetry.
    Answer {
        /// What to say.
        text: String,
        /// Identifiers of the knowledge entries this drew on, for the record.
        /// Empty where the model answered from context rather than a note.
        #[serde(default)]
        drew_on: Vec<String>,
    },
}

/// Work out what a person wants. **This is the model.**
///
/// The hardest work in the system happens here. What this has to do:
///
/// - **Understand** an utterance that is ambiguous, elliptical, hedged, or in
///   the middle of a conversation. "A bit less bitter than the one you made me
///   before the beans changed."
/// - **Resolve references** against what the machine remembers. Which earlier
///   cycle is "the one my wife liked"? Was that one actually good?
/// - **Decide how to express it**: a stated quantity where the person gave one,
///   a relation where they did not. Both are available; choosing well is
///   judgment.
/// - **Know when it does not know**, and ask. See [`Understood::Clarify`].
///
/// It is given the whole [`SelectionContext`], memory included, because
/// reasoning is its job and reasoning needs evidence. See ADI-P 9.4.1.
///
/// What it does *not* do is produce an actuation value, and that is the only
/// restriction on it. Its output is [`Intent`], a closed schema describing an
/// outcome rather than an action, per ADI-P 9.3 requirement 3. A malformed
/// intent is refused at the boundary rather than acted on, so a poor model
/// degrades the machine to not understanding, never to doing the wrong thing.
/// That is what lets this seam take the best model available.
///
/// The utterance arrives as text. Transcription sits in front of it, in
/// `adi-language`, and is a different model with a different failure mode.
pub trait Interpreter {
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError>;
}

/// The interpreter on a build with no model. Refuses, and says why.
///
/// Distinct from [`Understood::Clarify`]: a model that asks a question has
/// understood enough to know what it is missing, and the machine is working.
/// This is a build with no understanding at all, which is a configuration
/// error and is reported as one.
///
/// Present so the request path can be assembled, tested and shipped before any
/// model exists, and so the failure is a clear refusal rather than a machine
/// that silently ignores speech.
/// A boxed interpreter is an interpreter.
///
/// So that a caller choosing its backend at run time, which is what a
/// configuration file means, can still satisfy a bound of `P: Interpreter`.
impl<P: Interpreter + ?Sized> Interpreter for &P {
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        (**self).interpret(utterance, context)
    }
}

impl<P: Interpreter + ?Sized> Interpreter for Box<P> {
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        (**self).interpret(utterance, context)
    }
}

pub struct NoInterpreter;

impl Interpreter for NoInterpreter {
    fn interpret(&self, _: &str, _: &SelectionContext<'_>) -> Result<Understood, DecisionError> {
        Err(DecisionError::NoModelBackend("intent"))
    }
}

/// Validate what the interpreter produced before the engine acts on it.
///
/// Applied to every interpreter's output, including a future model's. The
/// engine never assumes it got it right. A question passes through untouched:
/// there is nothing in it to validate, because it reaches nothing.
pub fn accept_intent(understood: Understood) -> Result<Understood, DecisionError> {
    if let Understood::Intent(intent) = &understood {
        intent
            .validate()
            .map_err(|e| DecisionError::MalformedSelection(e.to_string()))?;
    }
    Ok(understood)
}

/// Build a selector for a level that needs no model backend.
///
/// Only D0 can be constructed this way, because the model-driven levels need a
/// [`ToolChooser`] or [`ActionChooser`]. Build those directly with
/// [`ModelToolSelector`] or [`ModelActionSelector::new`], which is also where
/// D2's stateful-envelope check happens.
///
/// The default level is D2, so this function refusing is the common case rather
/// than the exception. It refuses rather than quietly falling back to D0,
/// because a decision record saying D0 when D2 was asked for is a lie in the
/// audit trail, and the audit trail is the product.
pub fn selector_for(
    level: AutonomyLevel,
    declaration: &Declaration,
) -> Result<Box<dyn ActionSelector>, DecisionError> {
    match level {
        AutonomyLevel::D0 => Ok(Box::new(EngineSelector)),
        AutonomyLevel::D1 => Err(DecisionError::NoModelBackend("D1")),
        AutonomyLevel::D2 => {
            // Report the harder failure first: without a stateful envelope this
            // level is refused regardless of whether a backend is available.
            if !declaration.has_stateful_envelope() {
                return Err(DecisionError::StatefulEnvelopeRequired);
            }
            Err(DecisionError::NoModelBackend("D2"))
        }
    }
}

/// Derive candidates, select, verify the plan, and compute every magnitude.
///
/// Takes a set of goals, not one. A correction supplies a single goal; a
/// request supplies one per variable the person asked about. The candidate list
/// is the union across all of them, which is what lets a plan reach the grinder
/// and the sweetener in the same breath.
///
/// This used to take one diagnosis and build one variable's candidate list, and
/// the consequence was subtle: every candidate on offer moved the same
/// variable, so ADI-P 9.5 rejected any plan longer than one step. The
/// multi-step machinery existed and nothing could ever reach it.
///
/// `current` is the present value of each parameter the plan touches, looked up
/// by parameter name. A step whose parameter is not in the map is refused
/// rather than assumed to start from zero.
pub fn decide(
    planner: &Planner<'_>,
    selector: &dyn ActionSelector,
    goals: &[Goal],
    context: &SelectionContext<'_>,
    current: &std::collections::BTreeMap<String, Quantity>,
) -> Result<Vec<Correction>, DecisionError> {
    let first = attempt(planner, selector, goals, context, current);

    // One retry, and only for a mistake the model could fix if told what it
    // was. Before this, a chooser that returned two steps moving the same
    // variable failed the cycle outright, having never been shown the rule it
    // broke.
    //
    // Retrying a deterministic selector is pointless: D0 would return the same
    // answer, so the attempt is wasted and the error arrives later than it
    // should.
    let Err(error) = first else {
        return first;
    };
    if selector.level() == AutonomyLevel::D0 || !error.is_worth_retrying() {
        return Err(error);
    }

    // Bound it at one. A backend that is broken rather than mistaken must
    // surface as a failure, not as a loop.
    let reason = error.to_string();
    let second = context.with_rejection(&reason);
    attempt(planner, selector, goals, &second, current)
}

fn attempt(
    planner: &Planner<'_>,
    selector: &dyn ActionSelector,
    goals: &[Goal],
    context: &SelectionContext<'_>,
    current: &std::collections::BTreeMap<String, Quantity>,
) -> Result<Vec<Correction>, DecisionError> {
    check_goals(goals)?;
    let variables: Vec<VariableId> = goals
        .iter()
        .filter_map(|g| g.target_variable.clone())
        .collect();
    let candidates = planner.candidates_for_all(&variables);
    let selection = selector.select(goals, &candidates, context)?;

    // Resolve every step against the candidate list before planning any of it,
    // so a plan that is wrong in its last step performs none of its first.
    let mut resolved = Vec::with_capacity(selection.steps.len());
    for step in &selection.steps {
        let chosen = candidates
            .iter()
            .find(|c| c.tool == step.tool && c.parameter == step.parameter)
            .ok_or_else(|| DecisionError::UndeclaredSelection(step.tool.0.clone()))?;
        resolved.push((step, chosen));
    }

    check_one_variable_at_a_time(planner, &resolved)?;

    let mut out = Vec::with_capacity(resolved.len());
    for (step, chosen) in resolved {
        let present = current.get(&chosen.parameter).copied().ok_or_else(|| {
            DecisionError::MalformedSelection(format!(
                "no present value for parameter '{}'",
                chosen.parameter
            ))
        })?;

        // The goal this step serves, found by the variable its candidate moves.
        // A step whose candidate answers no goal cannot be planned, because the
        // desired change is what sizes it.
        let goal = goals
            .iter()
            .find(|g| g.target_variable.as_ref() == Some(&chosen.variable))
            .ok_or_else(|| DecisionError::UndeclaredSelection(chosen.tool.0.clone()))?;

        let correction = match step.value {
            Some(value) => {
                let correction = planner.plan_value(&chosen.variable, chosen, present, value)?;
                changes_something(&correction, chosen)?;
                correction
            }
            None => planner.plan(goal, chosen, present)?,
        };
        out.push(correction);
    }
    Ok(out)
}

/// Refuse a correction that would alter nothing, and say which kind it is.
///
/// Only D2 reaches this, because only D2 supplies the value. At D0 and D1 the
/// engine sizes the correction from the desired change and the mechanism's
/// response, and a small or clamped result there is the arithmetic working.
/// A value chosen elsewhere is a different matter: equal to the setting in
/// force, it moves nothing, and the engine would otherwise send it, record a
/// correction, and explain the fault it did not correct. The machine then
/// reports having acted on every cycle while the fault persists, which is a
/// worse failure than refusing, because refusing is visible.
///
/// `Range::exceeds` already states the same principle for the neighbouring
/// case: a value held at the end of its range is recorded as clamped, so that
/// a person can tell a small adjustment from no adjustment at all. This is the
/// case where there was no adjustment.
///
/// The delta is compared against zero exactly rather than against a tolerance.
/// `plan_value` has already snapped the value to the declared resolution, so a
/// proposal that rounds onto the present setting arrives here as exactly zero,
/// and a proposal that moves the parameter onto its grid from off it is a real
/// change that should not be refused.
///
/// Both refusals are `MalformedSelection`, which ADI-P 9.4.6 permits one
/// further attempt at, and the reason reaches the model as `rejected`. The two
/// messages differ because the corrections differ: one asks for a value that
/// moves, and one says this mechanism has nothing left to give and another
/// candidate is needed.
fn changes_something(correction: &Correction, chosen: &Candidate) -> Result<(), DecisionError> {
    if correction.delta.value != 0.0 {
        return Ok(());
    }

    if correction.clamped_from.is_some() {
        return Err(DecisionError::MalformedSelection(format!(
            "'{}' is already at the end of its declared range, so the value \
             proposed cannot move '{}' any further. Choose a different \
             candidate, or return no steps if none can serve the goal.",
            chosen.parameter, chosen.variable.0
        )));
    }

    Err(DecisionError::MalformedSelection(format!(
        "the value proposed for '{}' is {} {:?}, which is the setting already \
         in force, so it would change nothing and '{}' would not move. Propose \
         the absolute setting the parameter should reach.",
        chosen.parameter, correction.absolute.value, correction.absolute.unit, chosen.variable.0
    )))
}

/// A single correction, which is the common case on the diagnosis path.
pub fn decide_one(
    planner: &Planner<'_>,
    selector: &dyn ActionSelector,
    diagnosis: &Diagnosis,
    context: &SelectionContext<'_>,
    current: &std::collections::BTreeMap<String, Quantity>,
) -> Result<Vec<Correction>, DecisionError> {
    decide(
        planner,
        selector,
        std::slice::from_ref(diagnosis),
        context,
        current,
    )
}

impl DecisionError {
    /// Whether a second attempt could plausibly succeed.
    ///
    /// True for a malformed answer: the model proposed something the contract
    /// refuses, and being told which rule it broke is usually enough.
    ///
    /// False for everything structural. No candidate serves the goal, the goals
    /// themselves are contradictory, the declaration carries no stateful
    /// envelope, no backend is attached: asking again cannot change any of
    /// these, and retrying would delay a failure the caller needs to see.
    fn is_worth_retrying(&self) -> bool {
        matches!(
            self,
            DecisionError::ConflictingSteps { .. }
                | DecisionError::MalformedSelection(_)
                | DecisionError::UndeclaredSelection(_)
                | DecisionError::EmptyPlan
        )
    }
}

/// ADI-P 9.5. Two tools whose effects reach the same variable must not be
/// invoked in one iteration, because the next cycle cannot then attribute the
/// change to either. Applies to corrections, where attribution is the whole
/// point of acting.
fn check_one_variable_at_a_time(
    planner: &Planner<'_>,
    resolved: &[(&PlanStep, &Candidate)],
) -> Result<(), DecisionError> {
    for i in 0..resolved.len() {
        let a = planner.affected_variables(resolved[i].1);
        for j in (i + 1)..resolved.len() {
            let b = planner.affected_variables(resolved[j].1);
            if let Some(shared) = a.iter().find(|v| b.contains(v)) {
                return Err(DecisionError::ConflictingSteps {
                    first: resolved[i].1.tool.0.clone(),
                    second: resolved[j].1.tool.0.clone(),
                    variable: shared.0.clone(),
                });
            }
        }
    }
    Ok(())
}

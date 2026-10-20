//! Asking a model which action to take.
//!
//! The interpreter turns speech into a request. This is the separate question
//! of which declared action serves that request, and at D2 what magnitude to
//! send. ADI-P 3.3 grades an engine by who answers it.
//!
//! The safety argument does not rest on the model answering well. A chooser
//! returns an **index into a list the engine built from the declaration**, so
//! it cannot name a tool the machine does not have, and at D2 the value it
//! proposes is clamped to the declared range, snapped to the declared
//! resolution, checked against the stateful envelope with the rest of the
//! plan, and validated again by the appliance. A wrong answer is a refused
//! plan.
//!
//! The protocol is a line of JSON each way, as for the other seams:
//!
//! ```text
//! -> {"goals":[{"variable":"shot_time","delta":4.0,"confidence":0.8}],
//!     "candidates":[{"index":0,"tool":"...","parameter":"setting",
//!                    "variable":"shot_time","per_unit":-1.4,
//!                    "confidence":0.75,"unit":"step"}],
//!     "level":"d2"}
//!
//! <- {"steps":[{"candidate":0,"value":21.0}]}        d2, with magnitudes
//! <- {"steps":[{"candidate":0}]}                     d1, tool only
//! ```
//!
//! One step per goal, in the same order. There is no way to abstain: a goal
//! that was asked for is either served or the whole plan is refused. A chooser
//! that could silently drop one would produce a plan that looks complete and
//! is not.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use adi_core::plan::Candidate;
use adi_decision::{ActionChooser, Goal, ProposedStep, SelectionContext, ToolChooser};
use serde::{Deserialize, Serialize};

use crate::ModelError;

// ------------------------------------------------------------- the messages

#[derive(Serialize)]
struct Request<'a> {
    goals: Vec<GoalLine>,
    candidates: Vec<CandidateLine<'a>>,
    /// So a program can answer both levels without being configured twice.
    level: &'static str,
    remembered: Vec<String>,
    /// The rules the answer is checked against.
    ///
    /// Sent with every request rather than left to a backend author to
    /// reproduce. A model asked to satisfy a constraint it was never shown will
    /// break it, and before this was sent the commonest break, two steps moving
    /// one variable, failed the cycle outright.
    constraints: Vec<&'static str>,
    /// What this seam is for. See [`CHOOSER_INSTRUCTION`].
    instruction: &'static str,
    /// The shape of a valid answer, narrowed to the candidates actually
    /// offered and to this level, in the form a constrained decoder takes.
    ///
    /// ADI-P 9.3 requirement 3 on the way out as well as on the way back:
    /// naming a candidate the engine did not supply becomes impossible to emit
    /// rather than emitted and refused.
    schema: serde_json::Value,
    /// Why the previous answer was refused. Absent on a first attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    rejected: Option<&'a str>,
}

#[derive(Serialize)]
struct GoalLine {
    variable: String,
    cause: String,
    /// Signed, in the variable's own unit. How far the profile wants it moved.
    delta: Option<f64>,
    confidence: f64,
}

#[derive(Serialize)]
struct CandidateLine<'a> {
    /// What the reply refers to. The whole safety boundary is that a chooser
    /// returns one of these rather than a name of its own.
    index: usize,
    tool: &'a str,
    parameter: &'a str,
    variable: &'a str,
    /// Change in the variable per unit of the parameter. Signed, so a chooser
    /// can tell which direction a tool moves things.
    per_unit: f64,
    confidence: f64,
    /// The parameter's unit, which is not the variable's. Grind position is in
    /// steps and moves shot time in seconds.
    unit: String,
    /// What the parameter is set to now, where the cycle recorded it.
    ///
    /// At D2 the value proposed is the absolute value to send, not a change,
    /// so a chooser that cannot see the present setting cannot propose a
    /// sensible one. Without this, a model proposing a delta has it read as an
    /// absolute and clamped to the end of the range.
    current: Option<f64>,
    /// The manufacturer's description, where they wrote one.
    ///
    /// Advisory. It may inform which candidate is picked and reaches no
    /// actuation value: every candidate here is already declared, and whatever
    /// is proposed is clamped and checked regardless.
    #[serde(skip_serializing_if = "Option::is_none")]
    description: &'a Option<String>,
    /// What this tool changes besides the variable it is offered for.
    ///
    /// The thing that actually separates two tools serving one outcome. The
    /// magnitudes say how far each moves it; this says what else moves with it.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    also_affects: Vec<&'a str>,
}

#[derive(Deserialize)]
struct Reply {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    candidate: usize,
    /// Absent at D1, where the engine computes the magnitude.
    #[serde(default)]
    value: Option<f64>,
}

// --------------------------------------------------------------- the process

/// What a valid answer must satisfy.
///
/// Facts about the contract, not persuasion. The engine enforces every one of
/// these after the fact; sending them lets a model satisfy them first.
/// What the seam is for, stated by the engine.
///
/// Held here with [`CONSTRAINTS`] and sent with every request. A backend that
/// carries its own copy of this carries one that can drift from the rules the
/// engine actually enforces, and the shipped Qwen backend's copy disagreed with
/// the constraint below about how a D2 value is arrived at.
pub const CHOOSER_INSTRUCTION: &str =
    "You choose which of an appliance's mechanisms to move, and by how much.

You are given one or more goals and a numbered list of candidates. Each
candidate is a mechanism the machine declares, already filtered to ones that
move the goal's variable. Answer with one JSON object:

  {\"steps\": [{\"candidate\": <number from the list>, \"value\": <number>}]}

Each candidate gives \"current\", its present setting, and \"per_unit\", how much
the variable moves per unit of the parameter, signed.

A candidate's \"also_affects\" says what else it moves, which is how two
mechanisms serving one goal differ. Prefer the one whose side effects suit the
goal, then the one with the higher confidence.

If \"rejected\" is present, your previous answer was refused for the reason
given. Fix that specific thing.

Every rule the answer is checked against is listed under \"constraints\".";

const CONSTRAINTS: &[&str] = &[
    "Return exactly one step per goal, in the same order as the goals.",
    "Choose each step by its candidate index. Do not invent a tool name.",
    "No two steps may move the same variable. ADI-P 9.5: the next cycle could not then attribute the change to either, and the plan is refused whole.",
    "Every step needs a value, and it is the absolute setting to send, not a change. The goal's delta is in the variable's unit and the parameter is in its own, so the two do not add: value = current + delta / per_unit. The sign of per_unit already says which way the parameter moves.",
    "Serve every goal or none. A plan missing a goal is refused.",
];

struct Pipe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    command: String,
}

/// A chooser that is an external program.
///
/// Serves both levels. `ToolChooser` reads the index and ignores the value;
/// `ActionChooser` requires both.
pub struct ProcessChooser {
    inner: Mutex<Pipe>,
    level: &'static str,
}

impl ProcessChooser {
    /// Start the program. `level` is "d1" or "d2", and is sent with every
    /// request so one program can serve either.
    pub fn start(command: &str, args: &[String], level: &'static str) -> Result<Self, ModelError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| ModelError::Backend(format!("could not start '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard input".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard output".into()))?;

        Ok(Self {
            inner: Mutex::new(Pipe {
                child,
                stdin,
                stdout: BufReader::new(stdout),
                command: command.to_string(),
            }),
            level,
        })
    }

    fn ask(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Vec<Step>, String> {
        let request = Request {
            goals: goals
                .iter()
                .map(|g| GoalLine {
                    variable: g
                        .target_variable
                        .as_ref()
                        .map(|v| v.0.clone())
                        .unwrap_or_default(),
                    cause: g.cause.clone(),
                    delta: g.desired_delta,
                    confidence: g.confidence,
                })
                .collect(),
            candidates: candidates
                .iter()
                .enumerate()
                .map(|(index, c)| CandidateLine {
                    index,
                    tool: c.tool.as_str(),
                    parameter: &c.parameter,
                    variable: c.variable.as_str(),
                    per_unit: c.per_unit,
                    confidence: c.confidence,
                    unit: format!("{:?}", c.param_unit).to_lowercase(),
                    current: present(context, &c.parameter),
                    description: &c.description,
                    also_affects: c.also_affects.iter().map(|v| v.as_str()).collect(),
                })
                .collect(),
            level: self.level,
            instruction: CHOOSER_INSTRUCTION,
            schema: crate::selection_schema(candidates.len(), self.level),
            constraints: CONSTRAINTS
                .iter()
                .copied()
                .filter(|rule| self.level == "d2" || !rule.starts_with("Every step"))
                .collect(),
            rejected: context.rejected,
            remembered: context
                .memory
                // No import needed: these are trait methods on a trait
                // object, which are in scope through the object itself.
                .map(|m| m.recent(3).iter().map(|r| r.cycle_id.clone()).collect())
                .unwrap_or_default(),
        };

        let mut line = serde_json::to_string(&request)
            .map_err(|e| format!("could not encode the request: {e}"))?;
        line.push('\n');

        let mut pipe = self
            .inner
            .lock()
            .map_err(|_| "the backend lock was poisoned")?;

        pipe.stdin
            .write_all(line.as_bytes())
            .and_then(|()| pipe.stdin.flush())
            .map_err(|e| format!("could not reach the program: {e}"))?;

        let mut raw = String::new();
        if pipe
            .stdout
            .read_line(&mut raw)
            .map_err(|e| format!("could not read the reply: {e}"))?
            == 0
        {
            return Err(format!("'{}' produced no output and exited", pipe.command));
        }

        let reply: Reply = serde_json::from_str(raw.trim()).map_err(|e| {
            format!(
                "the reply is not a valid selection: {e}; got {}",
                raw.trim()
            )
        })?;

        // Checked here as well as in the selector, because an index out of
        // range would otherwise panic rather than refuse. The selector checks
        // the count; this checks that each index names a real candidate.
        for step in &reply.steps {
            if step.candidate >= candidates.len() {
                return Err(format!(
                    "candidate {} was chosen but only {} were offered",
                    step.candidate,
                    candidates.len()
                ));
            }
        }

        Ok(reply.steps)
    }
}

/// What a parameter is set to now.
///
/// Matched by name, or by a variable name ending with the parameter name. A
/// cycle records `grind_setting` while the tool that moves it declares a
/// parameter called `setting`, and the engine resolves the two the same way.
///
/// The engine's own record of the settings comes first, because it carries
/// what the appliance has reported and what the engine has sent, in the order
/// those happened. The cycle is the fallback and is older by definition: it
/// says what the machine was set to while it ran.
///
/// This read the cycle alone, and a request has no cycle, so at D2 a model
/// was asked for an absolute setting without being told the present one on
/// every spoken request.
fn present(context: &SelectionContext<'_>, parameter: &str) -> Option<f64> {
    if let Some(settings) = context.settings {
        if let Some(found) = settings
            .iter()
            .find(|(name, _)| name.as_str() == parameter || name.ends_with(parameter))
        {
            return Some(found.1.value);
        }
    }

    let cycle = context.cycle?;

    cycle
        .settings
        .iter()
        .find(|(variable, _)| variable.0 == parameter || variable.0.ends_with(parameter))
        .map(|(_, quantity)| quantity.value)
}

impl ToolChooser for ProcessChooser {
    fn choose(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Vec<usize>, String> {
        Ok(self
            .ask(goals, candidates, context)?
            .into_iter()
            .map(|s| s.candidate)
            .collect())
    }
}

impl ActionChooser for ProcessChooser {
    fn choose(
        &self,
        goals: &[Goal],
        candidates: &[Candidate],
        context: &SelectionContext<'_>,
    ) -> Result<Vec<ProposedStep>, String> {
        self.ask(goals, candidates, context)?
            .into_iter()
            .map(|s| {
                // At D2 the magnitude is the model's to propose. A step
                // without one is not a D2 answer, and defaulting it would
                // silently turn D2 into D1 while the record still said D2.
                s.value
                    .map(|value| ProposedStep {
                        candidate: s.candidate,
                        value,
                    })
                    .ok_or_else(|| {
                        format!(
                            "candidate {} was chosen with no value; D2 requires one",
                            s.candidate
                        )
                    })
            })
            .collect()
    }
}

impl Drop for ProcessChooser {
    fn drop(&mut self) {
        if let Ok(mut pipe) = self.inner.lock() {
            let _ = pipe.child.kill();
            let _ = pipe.child.wait();
        }
    }
}

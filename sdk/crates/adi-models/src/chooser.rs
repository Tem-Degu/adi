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
                })
                .collect(),
            level: self.level,
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

        let mut pipe = self.inner.lock().map_err(|_| "the backend lock was poisoned")?;

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
            return Err(format!(
                "'{}' produced no output and exited",
                pipe.command
            ));
        }

        let reply: Reply = serde_json::from_str(raw.trim())
            .map_err(|e| format!("the reply is not a valid selection: {e}; got {}", raw.trim()))?;

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

/// What a parameter is set to now, from the cycle being corrected.
///
/// Matched by name, or by a variable name ending with the parameter name. A
/// cycle records `grind_setting` while the tool that moves it declares a
/// parameter called `setting`, and the engine resolves the two the same way.
fn present(context: &SelectionContext<'_>, parameter: &str) -> Option<f64> {
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

//! A model that is another program.
//!
//! The seam is a trait, so a model written in anything can serve it provided
//! someone writes the Rust to call it. This removes that proviso: the program
//! reads a request on its standard input and writes an answer on its standard
//! output, and needs to know nothing about this SDK.
//!
//! The same pattern drives a real `libadip` build from the engine's tests. It
//! costs little and it is the difference between "you may use our models" and
//! "you may use any model".
//!
//! One line in, one line out, both JSON:
//!
//! ```text
//! -> {"utterance":"half of last time","styles":[...],"remembered":[...]}
//! <- {"intent":{"kind":"produce","targets":{...}}}
//! ```
//!
//! The reply is exactly what `schema::UNDERSTOOD_SCHEMA` describes. A program
//! that cannot resolve the request replies with a question, which is an
//! ordinary outcome rather than a failure.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use adi_decision::{accept_intent, DecisionError, Interpreter, SelectionContext, Understood};
use adi_language::context::{ContextBudget, ModelContext};
use serde::Serialize;

use crate::ModelError;

/// What the program is sent. A flattened `ModelContext`, so a program in any
/// language can read it without knowing our types.
#[derive(Debug, Serialize)]
struct Request<'a> {
    utterance: &'a str,
    styles: &'a [String],
    materials: &'a [String],
    variables: &'a [String],
    remembered: Vec<RememberedLine>,
    history: Vec<HistoryLine>,
}

#[derive(Debug, Serialize)]
struct RememberedLine {
    cycle_id: String,
    style: Option<String>,
    settings: Vec<String>,
    feedback: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HistoryLine {
    cycle_id: String,
    cause: String,
    tool: Option<String>,
}

/// An interpreter that is an external program.
///
/// The child is started once and kept, because starting a process per request
/// would pay model loading on every utterance, and loading is the expensive
/// part of a language model.
pub struct ProcessInterpreter {
    inner: Mutex<Pipe>,
    budget: ContextBudget,
    knowledge_styles: Vec<String>,
    knowledge_materials: Vec<String>,
    variables: Vec<String>,
}

struct Pipe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ProcessInterpreter {
    /// Start the program.
    ///
    /// What the model is told it may name is supplied here rather than read
    /// per request, because it does not change within a session and sending it
    /// every time would be paid on every utterance.
    pub fn start(
        command: &str,
        args: &[String],
        styles: Vec<String>,
        materials: Vec<String>,
        variables: Vec<String>,
        budget: ContextBudget,
    ) -> Result<Self, ModelError> {
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
            }),
            budget,
            knowledge_styles: styles,
            knowledge_materials: materials,
            variables,
        })
    }

    fn exchange(&self, request: &Request<'_>) -> Result<Understood, ModelError> {
        let mut line = serde_json::to_string(request)
            .map_err(|e| ModelError::Backend(format!("could not encode the request: {e}")))?;
        line.push('\n');

        let mut pipe = self
            .inner
            .lock()
            .map_err(|_| ModelError::Backend("the backend lock was poisoned".into()))?;

        pipe.stdin
            .write_all(line.as_bytes())
            .and_then(|()| pipe.stdin.flush())
            .map_err(|e| ModelError::Backend(format!("could not reach the program: {e}")))?;

        let mut reply = String::new();
        let read = pipe
            .stdout
            .read_line(&mut reply)
            .map_err(|e| ModelError::Backend(format!("could not read the reply: {e}")))?;

        if read == 0 {
            return Err(ModelError::Backend("the program closed its output".into()));
        }

        // A program that returns something outside the schema is refused
        // rather than guessed at. This is the boundary 9.3 requirement 3
        // describes, and it holds whether or not the program constrained its
        // own decoding.
        serde_json::from_str(reply.trim()).map_err(|e| {
            ModelError::Backend(format!(
                "the reply is not a valid answer: {e}; got {}",
                reply.trim()
            ))
        })
    }
}

impl Interpreter for ProcessInterpreter {
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        let mut assembled = ModelContext {
            utterance: utterance.to_string(),
            styles: self.knowledge_styles.clone(),
            materials: self.knowledge_materials.clone(),
            variables: self.variables.clone(),
            ..Default::default()
        };

        if let Some(memory) = context.memory {
            assembled.remembered = memory
                .recent(self.budget.remembered_cycles)
                .into_iter()
                .map(|entry| adi_language::context::RememberedSummary {
                    cycle_id: entry.cycle_id.clone(),
                    style: entry.style.clone(),
                    settings: entry
                        .settings
                        .iter()
                        .map(|(v, q)| format!("{}={}{:?}", v.0, q.value, q.unit))
                        .collect(),
                    feedback: entry.feedback.descriptors.clone(),
                })
                .collect();
        }

        assembled.history = context
            .history
            .iter()
            .take(self.budget.history)
            .map(|record| adi_language::context::DecisionSummary {
                cycle_id: record.cycle_id.clone(),
                cause: record.cause.clone(),
                tool: record.selected_tool.as_ref().map(|t| t.0.clone()),
            })
            .collect();

        let request = Request {
            utterance: &assembled.utterance,
            styles: &assembled.styles,
            materials: &assembled.materials,
            variables: &assembled.variables,
            remembered: assembled
                .remembered
                .iter()
                .map(|r| RememberedLine {
                    cycle_id: r.cycle_id.clone(),
                    style: r.style.clone(),
                    settings: r.settings.clone(),
                    feedback: r.feedback.clone(),
                })
                .collect(),
            history: assembled
                .history
                .iter()
                .map(|h| HistoryLine {
                    cycle_id: h.cycle_id.clone(),
                    cause: h.cause.clone(),
                    tool: h.tool.clone(),
                })
                .collect(),
        };

        let understood = self
            .exchange(&request)
            .map_err(|e| DecisionError::Backend(e.to_string()))?;

        // The same validation every interpreter's output passes through. A
        // program is not trusted more than a model linked into this binary.
        accept_intent(understood)
    }
}

impl Drop for ProcessInterpreter {
    fn drop(&mut self) {
        if let Ok(mut pipe) = self.inner.lock() {
            let _ = pipe.child.kill();
            let _ = pipe.child.wait();
        }
    }
}

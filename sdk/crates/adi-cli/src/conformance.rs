//! Run a conformance corpus against the engine.
//!
//! The corpus is data, not code. When the engine is rewritten, ported, or
//! reimplemented in another language, these files transfer unchanged and prove
//! the new build behaves identically. That is what makes the certification
//! mark mean something rather than being a claim.
//!
//! Cases run at autonomy D0 deliberately. A conformance result has to be
//! reproducible, and D1 puts a model in the selection path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adi_core::cycle::{Cycle, SubjectiveFeedback, Trace};
use adi_core::domain::Domain;
use adi_core::plan::Planner;
use adi_decision::{ActionSelector, EngineSelector};
use adi_domain_espresso::Espresso;
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::units::{Quantity, Unit};
use adi_protocol::{Declaration, Skill};
use serde::Deserialize;

// ------------------------------------------------------------------ corpus

#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub profile: String,
    pub declaration: String,
    #[serde(default)]
    pub skill: Option<String>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    pub id: String,
    #[serde(default)]
    pub style: Option<String>,
    /// Input material, if the case exercises a prior. Optional: most cases care
    /// about the style alone.
    #[serde(default)]
    pub material: Option<String>,
    /// A spoken request rather than a completed cycle. The engine has no
    /// request path yet, so such a case must declare `pending`.
    #[serde(default)]
    pub request: Option<String>,
    /// The structured form of `request`. What a model would have produced, and
    /// what the deterministic path below actually consumes.
    #[serde(default)]
    pub intent: Option<adi_protocol::Intent>,
    /// Why this case cannot run yet. Reported rather than silently skipped.
    #[serde(default)]
    pub pending: Option<String>,
    #[serde(default)]
    pub settings: BTreeMap<String, QtyLit>,
    #[serde(default)]
    pub trace: Option<TraceLit>,
    #[serde(default)]
    pub feedback: Option<FeedbackLit>,
    pub expect: Expect,
    /// Events the appliance reported during the cycle, such as when flow
    /// began. Optional: without them a profile falls back to estimating.
    #[serde(default)]
    events: Vec<CaseEvent>,
}

#[derive(Debug, Deserialize)]
struct CaseEvent {
    id: adi_protocol::ids::SemanticId,
    at_s: f64,
}

#[derive(Debug, Deserialize)]
pub struct QtyLit {
    pub value: f64,
    pub unit: Unit,
}

#[derive(Debug, Deserialize)]
pub struct TraceLit {
    pub signal: SemanticId,
    pub samples: Vec<(f64, f64)>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FeedbackLit {
    #[serde(default)]
    pub descriptors: Vec<String>,
    #[serde(default)]
    pub rating: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct Expect {
    pub cause: String,
    #[serde(default)]
    pub target_variable: Option<VariableId>,
    #[serde(default)]
    pub desired_delta_sign: Option<Sign>,
    #[serde(default)]
    pub selected_tool: Option<SemanticId>,
    #[serde(default)]
    pub direction: Option<Direction>,
    /// Variables the chosen tool must not affect. This is how a case asserts
    /// that a preference request never reaches the extraction.
    #[serde(default)]
    pub must_not_touch: Vec<VariableId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sign {
    Positive,
    Negative,
}

/// Direction of a correction, in the domain's own words.
///
/// The runner maps this to the sign of the change applied to the setting. For
/// espresso, finer is a decrease in the grind position. A domain where that
/// mapping differs needs its own vocabulary here rather than reusing this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Finer,
    Coarser,
}

// ------------------------------------------------------------------ result

#[derive(Debug)]
enum Outcome {
    Pass,
    Fail(Vec<String>),
    Pending(String),
}

// ------------------------------------------------------------------- runner

fn resolve(base: &Path, rel: &str) -> PathBuf {
    base.parent().unwrap_or(Path::new(".")).join(rel)
}

fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn domain_for(profile: &str) -> Result<Box<dyn Domain>, String> {
    match profile {
        "espresso/v1" => Ok(Box::new(Espresso::new())),
        other => Err(format!(
            "no domain profile '{other}' is compiled into this build"
        )),
    }
}

fn build_cycle(
    case: &Case,
    declaration: &Declaration,
) -> Result<Cycle, String> {
    let mut settings = BTreeMap::new();
    for (name, q) in &case.settings {
        settings.insert(
            VariableId::from(name.as_str()),
            Quantity::new(q.value, q.unit),
        );
    }

    let mut traces = Vec::new();
    if let Some(t) = &case.trace {
        // Accuracy comes from the declaration, not from the case. A corpus
        // that could invent its own accuracy could make a noisy signal look
        // decisive.
        let signal = declaration.signal(&t.signal).ok_or_else(|| {
            format!("signal '{}' is not declared by the machine", t.signal)
        })?;
        traces.push(Trace {
            signal: t.signal.clone(),
            samples: t.samples.clone(),
            unit: signal.unit,
            accuracy_abs: signal.accuracy.abs,
            filtered: signal.filtered,
        });
    }

    let feedback = case
        .feedback
        .as_ref()
        .map(|f| SubjectiveFeedback {
            descriptors: f.descriptors.clone(),
            rating: f.rating,
            note: None,
        })
        .unwrap_or_default();

    // A case may state when flow began, as an appliance would report it. When
    // it does not, the profile estimates it from the curve, and a case that
    // means to test the estimate says nothing here.
    let events = case
        .events
        .iter()
        .map(|e| adi_core::ObservedEvent {
            id: e.id.clone(),
            at_s: e.at_s,
        })
        .collect();

    Ok(Cycle {
        cycle_id: case.id.clone(),
        subject_id: None,
        operator_id: None,
        settings,
        measured: BTreeMap::new(),
        traces,
        events,
        feedback,
    })
}

/// The request path: a person asked for something, rather than a cycle having
/// gone wrong.
///
/// The case's `settings` are seeded as the remembered previous cycle, which is
/// what "half of last time" resolves against.
fn run_request_case(
    case: &Case,
    declaration: &Declaration,
    skill: Option<&Skill>,
    domain: &dyn Domain,
    intent: &adi_protocol::Intent,
    cycle: &adi_core::Cycle,
) -> Outcome {
    let mut problems: Vec<String> = Vec::new();

    let mut memory = adi_core::RecentCycles::default();
    memory.push(adi_core::Remembered {
        cycle_id: "previous".into(),
        style: case.style.clone(),
        settings: cycle.settings.clone(),
        feedback: cycle.feedback.clone(),
    });

    let resolved = match adi_core::request::resolve(intent, &memory) {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(vec![format!("resolving intent: {e}")]),
    };

    let Some((variable, target)) = resolved.targets.iter().next() else {
        return Outcome::Fail(vec!["the intent resolved to no targets".into()]);
    };

    let knowledge = domain.knowledge();
    let current = cycle.settings.get(variable).copied();
    let diagnosis = adi_core::request::as_diagnosis(variable, *target, current, knowledge);

    if diagnosis.cause != case.expect.cause {
        problems.push(format!(
            "cause: expected '{}', got '{}'",
            case.expect.cause, diagnosis.cause
        ));
    }
    if diagnosis.target_variable != case.expect.target_variable {
        problems.push(format!(
            "target_variable: expected {:?}, got {:?}",
            case.expect.target_variable.as_ref().map(|v| v.as_str()),
            diagnosis.target_variable.as_ref().map(|v| v.as_str())
        ));
    }

    let planner = Planner::new(declaration, skill, domain);

    // The rule this case exists to prove: a change to a preference variable
    // must not be achieved with a tool that also moves the process.
    let candidates = match adi_core::request::candidates_for(&planner, knowledge, variable) {
        Ok(c) => c,
        Err(e) => {
            problems.push(format!("no usable tool: {e}"));
            return Outcome::Fail(problems);
        }
    };

    for candidate in &candidates {
        for forbidden in &case.expect.must_not_touch {
            if planner.affected_variables(candidate).contains(forbidden) {
                problems.push(format!(
                    "'{}' affects '{forbidden}', which this case forbids",
                    candidate.tool
                ));
            }
        }
    }

    if let Some(want_tool) = &case.expect.selected_tool {
        // The same memory the resolver read. Both sides see it.
        let context = adi_decision::SelectionContext::default().with_memory(&memory);
        match EngineSelector.select(std::slice::from_ref(&diagnosis), &candidates, &context) {
            Err(e) => problems.push(format!("selection failed: {e}")),
            Ok(selection) => {
                let step = &selection.steps[0];
                if &step.tool != want_tool {
                    problems.push(format!(
                        "selected_tool: expected '{want_tool}', got '{}'",
                        step.tool
                    ));
                }
            }
        }
    }

    if problems.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(problems)
    }
}

fn run_case(
    case: &Case,
    declaration: &Declaration,
    skill: Option<&Skill>,
    domain: &dyn Domain,
    interpreter: Option<&dyn adi_decision::Interpreter>,
) -> Outcome {
    if let Some(reason) = &case.pending {
        return Outcome::Pending(reason.clone());
    }

    // A case may carry `request` as the words a person said and `intent` as
    // the structured form of them. Turning the first into the second is the
    // model's job, so a case with words and no intent needs an interpreter.
    // Without one it stays pending rather than appearing to pass.
    let interpreted;
    let effective_intent = match (&case.request, &case.intent) {
        (_, Some(intent)) => Some(intent),
        (Some(words), None) => {
            let Some(interpreter) = interpreter else {
                return Outcome::Pending(
                    "carries spoken words but no `intent`. Supply an interpreter with --interpreter, or add the structured intent to the case."
                        .to_string(),
                );
            };

            let context = adi_decision::SelectionContext {
                history: &[],
                cycle: None,
                memory: None,
            };

            match interpreter.interpret(words, &context) {
                Ok(adi_decision::Understood::Intent(intent)) => {
                    interpreted = intent;
                    Some(&interpreted)
                }
                // Asking is an ordinary outcome for an interpreter, but a case
                // expecting an action did not get one. That is a failure of
                // this case rather than of the interpreter.
                Ok(adi_decision::Understood::Clarify { question, because }) => {
                    return Outcome::Fail(vec![format!(
                        "the interpreter asked '{question}' rather than acting ({because})"
                    )]);
                }
                Err(e) => return Outcome::Fail(vec![format!("interpreting '{words}': {e}")]),
            }
        }
        (None, None) => None,
    };

    let mut problems: Vec<String> = Vec::new();

    let mut cycle = match build_cycle(case, declaration) {
        Ok(c) => c,
        Err(e) => return Outcome::Fail(vec![e]),
    };

    domain.derive(&mut cycle);

    if let Some(intent) = effective_intent {
        return run_request_case(case, declaration, skill, domain, intent, &cycle);
    }

    // Resolved from the knowledge base, so editing `knowledge.yaml` changes what
    // these cases produce. An unknown style is a failure rather than a fallback:
    // the previous hardcoded targets returned an espresso for any name they did
    // not recognise, which made every case pass while ignoring its style.
    let Some(style) = case.style.as_deref() else {
        return Outcome::Fail(vec!["no style declared".to_string()]);
    };
    let targets = match domain.knowledge().resolve(style, case.material.as_deref()) {
        Ok(resolution) => resolution.targets,
        Err(e) => return Outcome::Fail(vec![format!("resolving '{style}': {e}")]),
    };
    let diagnosis = domain.diagnose(&cycle, &targets);

    // ---------------------------------------------------------- diagnosis
    if diagnosis.cause != case.expect.cause {
        problems.push(format!(
            "cause: expected '{}', got '{}'",
            case.expect.cause, diagnosis.cause
        ));
    }

    if diagnosis.target_variable != case.expect.target_variable {
        problems.push(format!(
            "target_variable: expected {:?}, got {:?}",
            case.expect.target_variable.as_ref().map(|v| v.as_str()),
            diagnosis.target_variable.as_ref().map(|v| v.as_str())
        ));
    }

    if let Some(want) = case.expect.desired_delta_sign {
        match diagnosis.desired_delta {
            None => problems.push(format!(
                "desired_delta: expected a {want:?} value, got none"
            )),
            Some(d) => {
                let got = if d < 0.0 { Sign::Negative } else { Sign::Positive };
                if got != want {
                    problems.push(format!(
                        "desired_delta: expected {want:?}, got {got:?} ({d})"
                    ));
                }
            }
        }
    }

    // ---------------------------------------------------------- correction
    match (&case.expect.selected_tool, &diagnosis.target_variable) {
        (None, _) => {
            // Nothing should be proposed. A diagnosis that wants a variable
            // moved while the case says no tool is a contradiction worth
            // reporting.
            if diagnosis.target_variable.is_some() && case.expect.target_variable.is_none() {
                problems.push(
                    "no tool expected, but the diagnosis asked for an actuation"
                        .to_string(),
                );
            }
        }
        (Some(want_tool), None) => {
            problems.push(format!(
                "expected tool '{want_tool}', but the diagnosis proposed no variable"
            ));
        }
        (Some(want_tool), Some(variable)) => {
            let planner = Planner::new(declaration, skill, domain);
            let candidates = planner.candidates(variable);

            if candidates.is_empty() {
                problems.push(format!(
                    "expected tool '{want_tool}', but nothing declared can move \
                     '{variable}'. Is the skill loaded?"
                ));
            } else {
                // Pinned to D0. The corpus asserts an exact expected decision,
                // which is only meaningful where the same inputs always give
                // the same answer. The engine's default level is D2; this is
                // the one place that deliberately is not it.
                let context = adi_decision::SelectionContext::default();
                match EngineSelector.select(std::slice::from_ref(&diagnosis), &candidates, &context) {
                    Err(e) => problems.push(format!("selection failed: {e}")),
                    Ok(selection) => {
                        // D0 always returns a single step.
                        let step = &selection.steps[0];
                        if &step.tool != want_tool {
                            problems.push(format!(
                                "selected_tool: expected '{want_tool}', got '{}'",
                                step.tool
                            ));
                        }

                        let chosen = candidates
                            .iter()
                            .find(|c| c.tool == step.tool && c.parameter == step.parameter);

                        if let Some(chosen) = chosen {
                            // The present value of the tool *parameter*, which
                            // is not the value of the variable the tool moves.
                            //
                            // This previously tried `cycle.value(variable)`
                            // first and fell back to the setting. Because a
                            // derived variable is always present, the fallback
                            // never ran, and the planner received a shot time in
                            // seconds as the current grind position in steps.
                            // The cases still passed, because the direction
                            // happened to come out right.
                            let current = cycle.settings.iter().find_map(|(k, v)| {
                                if k.as_str() == chosen.parameter
                                    || k.as_str().ends_with(&chosen.parameter)
                                {
                                    Some(*v)
                                } else {
                                    None
                                }
                            });

                            let Some(current) = current else {
                                problems.push(format!(
                                    "the case declares no setting for parameter '{}', so there \
                                     is no present value to correct from",
                                    chosen.parameter
                                ));
                                return Outcome::Fail(problems);
                            };

                            match planner.plan(&diagnosis, chosen, current) {
                                Err(e) => problems.push(format!("planning failed: {e}")),
                                Ok(correction) => {
                                    if let Some(dir) = case.expect.direction {
                                        let got = if correction.delta.value < 0.0 {
                                            Direction::Finer
                                        } else {
                                            Direction::Coarser
                                        };
                                        if got != dir {
                                            problems.push(format!(
                                                "direction: expected {dir:?}, got {got:?} \
                                                 (delta {})",
                                                correction.delta.value
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        // must_not_touch: the chosen tool's declared effects
                        // must not include any of these.
                        if !case.expect.must_not_touch.is_empty() {
                            if let Some(tool) = declaration.tool(&step.tool) {
                                for forbidden in &case.expect.must_not_touch {
                                    if tool.effects.contains(forbidden) {
                                        problems.push(format!(
                                            "'{}' affects '{forbidden}', which this case \
                                             forbids",
                                            step.tool
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if problems.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(problems)
    }
}

/// Run every corpus file under `target`, which may be a file or a directory.
///
/// Returns the number of failures, so a caller can set the exit code.
/// Run the corpus.
///
/// `interpreter_script` supplies answers for cases carrying spoken words
/// rather than structured intent. Scripted rather than a model on purpose:
/// this proves the path from words to a moving mechanism, so that when a model
/// is substituted a failure is attributable to the model rather than to
/// everything around it.
pub fn run(target: &Path, interpreter_script: Option<&Path>) -> Result<usize, String> {
    let interpreter: Option<Box<dyn adi_decision::Interpreter>> = match interpreter_script {
        Some(path) => Some(Box::new(
            adi_models::ScriptedInterpreter::load(path)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        )),
        None => None,
    };

    let mut files: Vec<PathBuf> = Vec::new();

    if target.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(target)
            .map_err(|e| format!("{}: {e}", target.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|s| s.to_str()) == Some("yaml")
            })
            .collect();
        entries.sort();
        files.extend(entries);
    } else {
        files.push(target.to_path_buf());
    }

    if files.is_empty() {
        return Err(format!("no corpus files under {}", target.display()));
    }

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut total_pending = 0usize;

    for file in &files {
        let corpus: Corpus = load(file)?;
        let declaration: Declaration = load(&resolve(file, &corpus.declaration))?;
        declaration
            .validate()
            .map_err(|e| format!("{}: {e}", corpus.declaration))?;

        let skill: Option<Skill> = match &corpus.skill {
            Some(s) => Some(load(&resolve(file, s))?),
            None => None,
        };

        let domain = domain_for(&corpus.profile)?;

        println!("{}", file.display());
        println!(
            "  profile {}   machine {} {}",
            corpus.profile, declaration.device.vendor, declaration.device.model
        );
        if skill.is_none() {
            println!("  no skill: only diagnosis can be checked, not correction");
        }
        println!();

        for case in &corpus.cases {
            match run_case(
                case,
                &declaration,
                skill.as_ref(),
                domain.as_ref(),
                interpreter.as_deref(),
            ) {
                Outcome::Pass => {
                    total_pass += 1;
                    println!("  PASS  {}", case.id);
                }
                Outcome::Pending(reason) => {
                    total_pending += 1;
                    println!("  PEND  {}", case.id);
                    println!("          {reason}");
                }
                Outcome::Fail(problems) => {
                    total_fail += 1;
                    println!("  FAIL  {}", case.id);
                    for p in problems {
                        println!("          {p}");
                    }
                }
            }
        }
        println!();
    }

    println!(
        "{} passed, {} failed, {} pending",
        total_pass, total_fail, total_pending
    );

    // Pending cases are reported but do not fail the run. They are declared
    // gaps rather than regressions, and hiding them would be worse than
    // either passing or failing them.
    Ok(total_fail)
}

//! `adi models bench`: what an interpreter costs, and how often it is right.
//!
//! Two numbers decide whether a model is usable on a given board, and neither
//! can be taken from a published benchmark. Accuracy figures elsewhere are
//! measured on other people's tasks, and latency figures on other people's
//! hardware with other people's prompts. This runs the corpus that matters on
//! the machine in front of you.
//!
//! Latency is wall clock, because it is the only measure every backend can
//! report and the only one a person waiting for a drink experiences. A backend
//! that can also report tokens per second may do so; this does not require it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use adi_decision::{Interpreter, SelectionContext, Understood};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    say: String,
    expect: Expectation,
}

#[derive(Debug, Deserialize)]
struct Expectation {
    /// The intent a correct interpreter produces. Compared field by field, so
    /// a case states only what it means to constrain.
    #[serde(default)]
    intent: Option<serde_json::Value>,
    /// Present when asking is the correct answer.
    #[serde(default)]
    clarify: Option<serde_json::Value>,
}

enum Verdict {
    Correct,
    /// Asked when it should have acted, or acted when it should have asked.
    WrongKind(String),
    /// Acted, but not as the case expects.
    WrongContent(String),
    Failed(String),
}

pub fn bench(
    corpus_path: PathBuf,
    interpreter_script: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(&corpus_path)
        .map_err(|e| format!("{}: {e}", corpus_path.display()))?;
    let corpus: Corpus = serde_yaml::from_str(&text)
        .map_err(|e| format!("{}: {e}", corpus_path.display()))?;

    let Some(script) = interpreter_script else {
        println!("no interpreter given.");
        println!();
        println!("  --interpreter <file.yaml>   scripted answers, which measure the");
        println!("                              engine with the model time removed");
        println!();
        println!("A model backend is configured in models.yaml and reported by");
        println!("`adi models check`. This command measures whichever is in use.");
        return Ok(());
    };

    let interpreter = adi_models::ScriptedInterpreter::load(&script)?;
    let label = format!("scripted ({})", script.display());

    run(&corpus, &interpreter, &label, &corpus_path)
}

fn run(
    corpus: &Corpus,
    interpreter: &dyn Interpreter,
    label: &str,
    corpus_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("  corpus     {}", corpus_path.display());
    println!("  backend    {label}");
    println!("  cases      {}", corpus.cases.len());
    println!();

    let mut timings: Vec<f64> = Vec::new();
    let mut correct = 0usize;
    let mut asked_correctly = 0usize;
    let mut expected_questions = 0usize;
    let mut problems: Vec<(String, String)> = Vec::new();

    let context = SelectionContext {
        history: &[],
        cycle: None,
        memory: None,
    };

    for case in &corpus.cases {
        if case.expect.clarify.is_some() {
            expected_questions += 1;
        }

        let started = Instant::now();
        let answer = interpreter.interpret(&case.say, &context);
        timings.push(started.elapsed().as_secs_f64() * 1000.0);

        match judge(case, answer) {
            Verdict::Correct => {
                correct += 1;
                if case.expect.clarify.is_some() {
                    asked_correctly += 1;
                }
            }
            Verdict::WrongKind(why) | Verdict::WrongContent(why) | Verdict::Failed(why) => {
                problems.push((case.id.clone(), why));
            }
        }
    }

    let total = corpus.cases.len();
    let percent = if total == 0 { 0.0 } else { correct as f64 * 100.0 / total as f64 };

    println!("  correct    {correct} of {total}  ({percent:.0}%)");
    if expected_questions > 0 {
        println!(
            "  asked      {asked_correctly} of {expected_questions} questions correctly asked"
        );
    }

    if !problems.is_empty() {
        println!();
        for (id, why) in &problems {
            println!("  WRONG  {id}");
            println!("         {why}");
        }
    }

    report_latency(&timings);

    println!();
    if label.starts_with("scripted") {
        println!("  Scripted answers measure the engine with the model time removed,");
        println!("  so the accuracy above reflects the script rather than a model.");
        println!("  The latency is the floor a real backend is added to.");
    }

    Ok(())
}

fn report_latency(timings: &[f64]) {
    if timings.is_empty() {
        return;
    }

    let mut sorted = timings.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let median = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() as f64 * 0.95) as usize % sorted.len()];
    let worst = sorted[sorted.len() - 1];
    let total: f64 = sorted.iter().sum();

    println!();
    println!("  latency    median {median:.1} ms   p95 {p95:.1} ms   worst {worst:.1} ms");
    println!("             {total:.0} ms over {} requests", sorted.len());

    // A person at a machine waits a couple of seconds before assuming it has
    // not heard them. Stated as a threshold rather than a judgement, so the
    // command reports rather than opines.
    const INTERACTIVE_MS: f64 = 2000.0;
    if p95 > INTERACTIVE_MS {
        println!();
        println!("  p95 exceeds {INTERACTIVE_MS:.0} ms. Consider a smaller model, a tighter");
        println!("  context budget, or a board with more memory bandwidth.");
    }
}

/// Compare an answer with what the case expects.
fn judge(case: &Case, answer: Result<Understood, adi_decision::DecisionError>) -> Verdict {
    let answer = match answer {
        Ok(a) => a,
        Err(e) => return Verdict::Failed(format!("the interpreter failed: {e}")),
    };

    match (&case.expect.intent, &case.expect.clarify, answer) {
        // Asking was correct, and it asked.
        (None, Some(_), Understood::Clarify { .. }) => Verdict::Correct,

        // Asking was correct, and it acted instead. The worst outcome in this
        // corpus: an ambiguous request became a physical action.
        (None, Some(_), Understood::Intent(_)) => {
            Verdict::WrongKind("acted on a request that should have been questioned".into())
        }

        // Acting was correct, and it asked. Cautious rather than dangerous,
        // but still wrong.
        (Some(_), None, Understood::Clarify { question, .. }) => {
            Verdict::WrongKind(format!("asked '{question}' rather than acting"))
        }

        // Acting was correct, and it acted. Now compare the content.
        (Some(expected), None, Understood::Intent(intent)) => {
            let produced = match serde_json::to_value(&intent) {
                Ok(v) => v,
                Err(e) => return Verdict::Failed(format!("could not compare: {e}")),
            };
            match subset_of(expected, &produced) {
                Ok(()) => Verdict::Correct,
                Err(why) => Verdict::WrongContent(why),
            }
        }

        _ => Verdict::Failed("the case states neither an intent nor a question".into()),
    }
}

/// Is every field the case states present, and equal, in what was produced?
///
/// A subset rather than equality, so a case constrains what it means to
/// constrain and says nothing about the rest. A case that wants to forbid a
/// field states it with the value it must have.
fn subset_of(expected: &serde_json::Value, produced: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;

    match (expected, produced) {
        (Value::Object(want), Value::Object(got)) => {
            for (key, value) in want {
                let Some(found) = got.get(key) else {
                    return Err(format!("expected '{key}', which is absent"));
                };
                subset_of(value, found).map_err(|inner| format!("{key}: {inner}"))?;
            }
            Ok(())
        }
        (Value::Number(want), Value::Number(got)) => {
            let (a, b) = (want.as_f64().unwrap_or(f64::NAN), got.as_f64().unwrap_or(f64::NAN));
            if (a - b).abs() < 1e-9 {
                Ok(())
            } else {
                Err(format!("expected {a}, got {b}"))
            }
        }
        _ if expected == produced => Ok(()),
        _ => Err(format!("expected {expected}, got {produced}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adi_protocol::intent::{Intent, IntentKind};

    fn case(id: &str, expect: Expectation) -> Case {
        Case {
            id: id.into(),
            say: "anything".into(),
            expect,
        }
    }

    fn expects_intent(json: serde_json::Value) -> Expectation {
        Expectation {
            intent: Some(json),
            clarify: None,
        }
    }

    fn expects_question() -> Expectation {
        Expectation {
            intent: None,
            clarify: Some(serde_json::json!({ "because": "ambiguous" })),
        }
    }

    fn produced(style: Option<&str>) -> Understood {
        Understood::Intent(Intent {
            kind: IntentKind::Produce,
            style: style.map(str::to_string),
            material: None,
            targets: Default::default(),
        })
    }

    fn asked() -> Understood {
        Understood::Clarify {
            question: "Which one?".into(),
            because: "ambiguous".into(),
        }
    }

    #[test]
    fn a_matching_intent_is_correct() {
        let c = case("x", expects_intent(serde_json::json!({ "kind": "produce" })));
        assert!(matches!(
            judge(&c, Ok(produced(Some("adi.example.style.standard")))),
            Verdict::Correct
        ));
    }

    /// The worst outcome the corpus can record. An ambiguous request became a
    /// physical action rather than a question, which is exactly what ADI-P
    /// 9.4.3 permits a model to avoid.
    #[test]
    fn acting_on_a_request_that_should_have_been_questioned_is_wrong() {
        let c = case("x", expects_question());
        match judge(&c, Ok(produced(None))) {
            Verdict::WrongKind(why) => assert!(why.contains("questioned"), "{why}"),
            _ => panic!("acting on an ambiguous request must not score as correct"),
        }
    }

    /// Cautious rather than dangerous, but still not what was asked for.
    #[test]
    fn asking_when_it_should_have_acted_is_wrong() {
        let c = case("x", expects_intent(serde_json::json!({ "kind": "produce" })));
        assert!(matches!(judge(&c, Ok(asked())), Verdict::WrongKind(_)));
    }

    #[test]
    fn asking_when_asking_was_right_is_correct() {
        let c = case("x", expects_question());
        assert!(matches!(judge(&c, Ok(asked())), Verdict::Correct));
    }

    #[test]
    fn a_wrong_field_is_reported_with_the_field_name() {
        let c = case(
            "x",
            expects_intent(serde_json::json!({ "style": "adi.example.style.other" })),
        );
        match judge(&c, Ok(produced(Some("adi.example.style.standard")))) {
            Verdict::WrongContent(why) => assert!(why.contains("style"), "{why}"),
            other => panic!("expected a content mismatch, got {}", describe(&other)),
        }
    }

    fn describe(v: &Verdict) -> String {
        match v {
            Verdict::Correct => "correct".into(),
            Verdict::WrongKind(w) => format!("wrong kind: {w}"),
            Verdict::WrongContent(w) => format!("wrong content: {w}"),
            Verdict::Failed(w) => format!("failed: {w}"),
        }
    }

    /// A case states only what it means to constrain. Anything the model adds
    /// that the case did not mention is not by itself an error.
    #[test]
    fn extra_fields_in_the_answer_are_permitted() {
        let expected = serde_json::json!({ "kind": "produce" });
        let got = serde_json::json!({ "kind": "produce", "style": "anything", "targets": {} });
        assert!(subset_of(&expected, &got).is_ok());
    }

    #[test]
    fn a_missing_field_is_named() {
        let expected = serde_json::json!({ "targets": { "cycle_time": { "scale": 0.5 } } });
        let got = serde_json::json!({ "targets": {} });
        let err = subset_of(&expected, &got).expect_err("should not match");
        assert!(err.contains("targets"), "{err}");
        assert!(err.contains("cycle_time"), "{err}");
    }

    /// Numbers are compared with a tolerance, because a model that emits 0.5
    /// and one that emits 0.50000000001 have both said the same thing.
    #[test]
    fn numbers_compare_within_a_tolerance() {
        assert!(subset_of(&serde_json::json!(0.5), &serde_json::json!(0.5)).is_ok());
        assert!(subset_of(&serde_json::json!(0.5), &serde_json::json!(0.6)).is_err());
    }

    #[test]
    fn a_backend_failure_is_not_a_wrong_answer() {
        let c = case("x", expects_intent(serde_json::json!({ "kind": "produce" })));
        let failed = Err(adi_decision::DecisionError::Backend("unreachable".into()));
        assert!(matches!(judge(&c, failed), Verdict::Failed(_)));
    }
}

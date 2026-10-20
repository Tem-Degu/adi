//! A model backend that is another program.
//!
//! The seams are traits, so a model written in anything can serve one provided
//! somebody writes the Rust to call it. The process backend removes that
//! proviso: a program reads a request on its standard input and writes an
//! answer on its standard output, and needs no dependency on this SDK.
//!
//! These tests drive `examples/interpreters/reference.py` as a real child
//! process. It is not a model and does not try to be. What it demonstrates is
//! that the boundary works, so a manufacturer can put anything behind it.
//!
//! Skipped rather than failed where Python is absent, so `cargo test` does not
//! require an interpreter to be installed.

use adi_language::context::VariableInfo;
use adi_decision::{Interpreter, SelectionContext, Understood};
use adi_language::context::ContextBudget;
use adi_models::ProcessInterpreter;

fn python() -> Option<&'static str> {
    for candidate in ["python3", "python"] {
        let ok = std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate);
        }
    }
    None
}

fn script() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/interpreters/reference.py")
        .to_string_lossy()
        .into_owned()
}

/// Two styles, one described and one not, since the knowledge may say nothing
/// about a style it declares.
fn styles() -> Vec<adi_language::context::StyleInfo> {
    vec![
        adi_language::context::StyleInfo {
            name: "adi.espresso.style.espresso".into(),
            description: Some("The standard shot.".into()),
        },
        adi_language::context::StyleInfo {
            name: "adi.espresso.style.lungo".into(),
            description: None,
        },
    ]
}

/// Two notes, both of which reach the model on every utterance.
fn notes() -> Vec<adi_protocol::knowledge::Note> {
    vec![
        adi_protocol::knowledge::Note {
            id: "x.example.note.basket".into(),
            text: "A 58 mm basket, taking 18 to 20 g.".into(),
        },
        adi_protocol::knowledge::Note {
            id: "x.example.note.water".into(),
            text: "Filtered water only; the reservoir has no softener.".into(),
        },
    ]
}

fn variables() -> Vec<VariableInfo> {
    // One described and one not, so the request carries both shapes and a
    // backend that assumes every variable has a meaning is caught here.
    vec![
        VariableInfo {
            name: "sugar_mass".into(),
            unit: Some("g".into()),
            meaning: Some("Sugar added to the drink. A preference.".into()),
        },
        VariableInfo {
            name: "shot_time".into(),
            unit: None,
            meaning: None,
        },
    ]
}

fn start() -> Option<ProcessInterpreter> {
    let python = python()?;
    ProcessInterpreter::start(
        python,
        &[script()],
        adi_models::Surroundings {
            styles: styles(),
            materials: Vec::new(),
            variables: variables(),
            notes: notes(),
            device: Some("Example Appliances EX-900, firmware 1.0.0".into()),
        },
        ContextBudget::default(),
    )
    .ok()
}

macro_rules! interpreter {
    () => {
        match start() {
            Some(i) => i,
            None => {
                eprintln!("skipping: no python on PATH");
                return;
            }
        }
    };
}

fn empty() -> SelectionContext<'static> {
    SelectionContext {
        history: &[],
        cycle: None,
        memory: None,
        rejected: None,
        settings: None,
    }
}

/// The first exchange across the boundary.
#[test]
fn a_program_can_serve_the_interpreter_seam() {
    let interpreter = interpreter!();

    match interpreter.interpret("make me an espresso", &empty()) {
        Ok(Understood::Intent(intent)) => {
            assert_eq!(
                intent.style.as_deref(),
                Some("adi.espresso.style.espresso"),
                "the program named a style the machine declares"
            );
        }
        other => panic!("expected an intent, got {other:?}"),
    }
}

/// ADI-P 9.4.3. A request the backend cannot resolve becomes a question, and a
/// question is an ordinary outcome rather than a failure.
#[test]
fn a_program_may_ask_instead_of_answering() {
    let interpreter = interpreter!();

    match interpreter.interpret("make it half", &empty()) {
        Ok(Understood::Clarify { question, because }) => {
            assert!(!question.is_empty(), "there is something to ask");
            assert!(
                because.contains("variable"),
                "and a reason for the record: {because}"
            );
        }
        other => panic!("expected a question, got {other:?}"),
    }
}

/// The relation is stated, not resolved. A backend that returned a number
/// would have to be trusted to remember correctly and to divide correctly, and
/// would be wrong silently when it did not. ADI-P 9.4.
#[test]
fn a_relative_request_crosses_as_a_relation() {
    use adi_core::{RecentCycles, Remembered, SubjectiveFeedback};
    use adi_protocol::ids::VariableId;
    use adi_protocol::units::{Quantity, Unit};

    let interpreter = interpreter!();

    let mut memory = RecentCycles::new(8);
    let mut settings = std::collections::BTreeMap::new();
    settings.insert(
        VariableId::from("sugar_mass"),
        Quantity::new(8.0, Unit::G),
    );
    memory.push(Remembered {
        cycle_id: "c1".into(),
        style: Some("adi.espresso.style.espresso".into()),
        settings,
        feedback: SubjectiveFeedback::default(),
    });
    assert_eq!(memory.len(), 1);

    let context = SelectionContext {
        history: &[],
        cycle: None,
        memory: Some(&memory),
        rejected: None,
        settings: None,
    };

    match interpreter.interpret("half the sugar of last time", &context) {
        Ok(Understood::Intent(intent)) => {
            let json = serde_json::to_value(&intent).expect("serialises");
            let target = json
                .get("targets")
                .and_then(|t| t.get("sugar_mass"))
                .expect("a target for the variable named");

            assert_eq!(target.get("of").unwrap(), "last", "a relation, not a value");
            assert_eq!(target.get("scale").unwrap(), 0.5);
            assert!(
                target.get("value").is_none(),
                "the backend must not resolve the arithmetic itself"
            );
        }
        other => panic!("expected a relative intent, got {other:?}"),
    }
}

/// A relation with nothing to relate to. The machine has produced nothing, so
/// there is no previous cycle to be half of, and inventing one would put a
/// fabricated value into a decision record.
#[test]
fn a_relation_with_no_memory_is_questioned() {
    let interpreter = interpreter!();

    match interpreter.interpret("the same as last time", &empty()) {
        Ok(Understood::Clarify { because, .. }) => {
            assert!(
                because.contains("previous") || because.contains("exists"),
                "the reason should say what is missing: {because}"
            );
        }
        other => panic!("expected a question, got {other:?}"),
    }
}

/// The process is started once and kept. Starting one per request would pay
/// model loading on every utterance, and loading is the expensive part of a
/// language model.
#[test]
fn the_program_survives_between_requests() {
    let interpreter = interpreter!();

    for n in 0..5 {
        let answer = interpreter.interpret("make me a lungo", &empty());
        assert!(
            matches!(answer, Ok(Understood::Intent(_))),
            "request {n} failed: {answer:?}"
        );
    }
}

/// Whatever a backend returns passes the same validation as a model linked
/// into this binary. Being a program is not a reason to be trusted.
#[test]
fn a_programs_answer_is_validated_like_any_other() {
    let interpreter = interpreter!();

    // The reference program only ever names styles it was given, so a request
    // for something undeclared must come back as a question rather than as an
    // intent naming a style the machine cannot make.
    match interpreter.interpret("make me a cortado please", &empty()) {
        Ok(Understood::Clarify { .. }) => {}
        Ok(Understood::Intent(intent)) => {
            assert!(
                intent
                    .style
                    .as_deref()
                    .is_none_or(|s| styles().iter().any(|declared| declared.name == s)),
                "a backend must not invent a style the machine does not declare"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

// ------------------------------------------------------ answering a question

/// ADI-P 9.4.3. A question is answered, and nothing is actuated.
///
/// Before this the model had two outcomes, act or ask, so a question came back
/// as an intent and the machine tried to do something, or as a clarification
/// and it asked a question of someone who had just asked one.
#[test]
fn a_question_about_the_machine_is_answered() {
    let Some(interpreter) = start() else {
        eprintln!("skipping: no python on PATH");
        return;
    };

    match interpreter.interpret("what model are you", &empty()) {
        Ok(Understood::Answer { text, drew_on }) => {
            assert!(text.contains("EX-900"), "it should say what it is: {text}");
            assert!(
                drew_on.is_empty(),
                "its own identity needs no note: {drew_on:?}"
            );
        }
        other => panic!("expected an answer, got {other:?}"),
    }
}

/// A question a note covers is answered from it, and the note is named.
#[test]
fn a_question_a_note_covers_names_the_note() {
    let Some(interpreter) = start() else {
        eprintln!("skipping: no python on PATH");
        return;
    };

    match interpreter.interpret("what size is the basket", &empty()) {
        Ok(Understood::Answer { text, drew_on }) => {
            assert!(text.contains("58"), "answered from the note: {text}");
            assert_eq!(
                drew_on,
                vec!["x.example.note.basket".to_string()],
                "and the source is recorded"
            );
        }
        other => panic!("expected an answer, got {other:?}"),
    }
}

/// A request is still a request. If every utterance became an answer the
/// machine would have stopped working, which a test asserting only the new
/// path would not notice.
#[test]
fn a_request_is_still_understood_as_an_intent() {
    let Some(interpreter) = start() else {
        eprintln!("skipping: no python on PATH");
        return;
    };

    match interpreter.interpret("make me an espresso", &empty()) {
        Ok(Understood::Intent(_)) => {}
        other => panic!("a request must still be an intent, got {other:?}"),
    }
}

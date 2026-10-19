//! The backends that are shipped with real models answer both seams.
//!
//! `reference.py` had eleven test files and `qwen_interpreter.py` had none, so
//! nothing noticed that the real-weights backend could not answer a selection
//! request at all: its schema made `steps` unemittable and its loop read
//! `utterance` unconditionally. Every test passed, against a stand-in that
//! happened to serve both seams, while D1 and D2 were broken for anyone who
//! downloaded the models.
//!
//! The same arrangement hid a second one for longer. The backend carried its
//! own copy of the output schema, and the copy permitted an intent and a
//! question but not an answer, so a machine running real weights met every
//! question with a question. The engine had permitted an answer throughout.
//! The contract is now sent with the request and the backend holds no copy, so
//! the fixtures below are built from the engine's own functions: a fixture
//! written out by hand is another copy, and a copy is a thing that drifts.
//!
//! These run without weights. `--explain` reports how a request would be
//! served and loads nothing, so CI can check the routing it cannot check the
//! judgement of. What the model decides, once it is decided correctly shaped,
//! is `adi models bench`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use adi_models::{narrowed_schema, selection_schema, SchemaVariable, OUTPUT_INSTRUCTION};

fn sdk() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn python() -> Option<&'static str> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate);
        }
    }
    None
}

/// Ask a backend how it would serve one request.
fn explain(script: &str, request: &str) -> Option<serde_json::Value> {
    let python = python()?;

    let mut child = Command::new(python)
        .arg(sdk().join("examples/interpreters").join(script))
        .arg("--explain")
        .current_dir(sdk())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the backend should start");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{request}\n").as_bytes())
        .unwrap();

    let out = child.wait_with_output().expect("should finish");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or_else(|| {
        panic!(
            "{script} --explain produced nothing.\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });

    Some(serde_json::from_str(line).expect("the explanation should be JSON"))
}

/// The shape `ProcessChooser` actually puts on the wire. Taken from
/// `adi-models/src/chooser.rs`, field for field: a goal carries `cause` and
/// `confidence` and no unit of its own, a candidate carries the `parameter` it
/// sets and that parameter's `unit`, and `remembered` holds the identifiers of
/// recent cycles. An earlier version of this fixture invented a goal unit and
/// omitted the rest, which is how a prompt can drop a field and still pass.
///
/// The instruction and the schema come from the engine, as they do in a real
/// request.
fn selection(level: &str) -> String {
    serde_json::json!({
        "goals": [{
            "variable": "shot_time",
            "cause": "under_extracted",
            "delta": 4.0,
            "confidence": 0.8
        }],
        "candidates": [
            {"index": 0, "tool": "adi.espresso.grind.set_setting", "parameter": "setting",
             "unit": "step", "variable": "shot_time", "per_unit": -1.4, "current": 24,
             "confidence": 0.75},
            {"index": 1, "tool": "adi.espresso.dose.set_mass", "parameter": "mass",
             "unit": "g", "variable": "shot_time", "per_unit": 0.6, "current": 18,
             "confidence": 0.5}
        ],
        "level": level,
        "remembered": ["cycle-0012", "cycle-0011"],
        "constraints": ["no two steps may move the same variable"],
        "instruction": adi_models::CHOOSER_INSTRUCTION,
        "schema": selection_schema(2, level),
    })
    .to_string()
}

/// What `ProcessInterpreter` puts on the wire for something someone said.
///
/// Carries the three fields that the prompt used to discard: the machine
/// identity, the manufacturer's notes, and the decisions already taken. ADI-P
/// 9.4.5 names the first two as what an answer is composed from.
fn utterance(said: &str) -> String {
    let styles = ["adi.espresso.style.espresso".to_string()];
    let variables = [SchemaVariable {
        name: "shot_time",
        unit: Some("s"),
    }];

    serde_json::json!({
        "utterance": said,
        "styles": styles,
        "materials": [],
        "variables": [{
            "name": "shot_time",
            "unit": "s",
            "meaning": "Seconds from first drip to stop."
        }],
        "notes": [{
            "id": "adi.espresso.note.sour_vs_bitter",
            "text": "Sour usually means under-extraction."
        }],
        "machine": "Example Appliances EX-900, firmware 3.2.1",
        "remembered": [],
        "history": [{
            "cycle_id": "cycle-0012",
            "cause": "under_extracted",
            "tool": "adi.espresso.grind.set_setting"
        }],
        "instruction": OUTPUT_INSTRUCTION,
        "schema": narrowed_schema(&styles, &[], &variables),
    })
    .to_string()
}

macro_rules! require_python {
    ($e:expr) => {
        match $e {
            Some(value) => value,
            None => return,
        }
    };
}

#[test]
fn a_selection_request_is_recognised_as_one() {
    let plan = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    assert_eq!(
        plan["seam"], "chooser",
        "a request carrying candidates is a selection"
    );
}

#[test]
fn a_spoken_request_is_still_an_utterance() {
    let plan = require_python!(explain(
        "qwen_interpreter.py",
        &utterance("half of last time")
    ));
    assert_eq!(plan["seam"], "interpreter");
}

/// The bug this file exists for. `steps` must be emittable.
#[test]
fn a_selection_can_actually_answer_with_steps() {
    let plan = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    let schema = &plan["schema"];

    assert!(
        schema["properties"]["steps"].is_object(),
        "a selection must be able to answer with steps, got {schema}"
    );
    assert_eq!(schema["required"][0], "steps");
}

/// The second bug this file exists for.
///
/// A machine that cannot answer a question is not a machine that answers
/// badly. The arm has to be present in the schema the decoder is given, or the
/// reply is unemittable and the model asks a question instead, which is what
/// shipped.
#[test]
fn an_answer_is_emittable() {
    let plan = require_python!(explain("qwen_interpreter.py", &utterance("what model are you")));
    let arms = plan["schema"]["oneOf"]
        .as_array()
        .unwrap_or_else(|| panic!("the schema should offer arms: {}", plan["schema"]));

    let has_answer = arms
        .iter()
        .any(|arm| arm["properties"].get("answer").is_some());

    assert!(
        has_answer,
        "ADI-P 9.4.5 permits an answer and the engine sends an arm for it: {arms:?}"
    );
}

/// ADI-P 3.3.1: a selector must not be able to name a tool the machine did
/// not declare. The engine refuses an unknown index on the way back; the
/// grammar makes one unemittable on the way out.
#[test]
fn the_candidate_is_limited_to_the_ones_offered() {
    let plan = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    let candidate = &plan["schema"]["properties"]["steps"]["items"]["properties"]["candidate"];

    assert_eq!(
        candidate["enum"],
        serde_json::json!([0, 1]),
        "only the offered indices may be named, got {candidate}"
    );
}

/// At D2 the value is the absolute setting and is required. At D1 the engine
/// sizes the correction, so a value sent there would be read as one it chose.
#[test]
fn a_value_is_required_at_d2_and_absent_at_d1() {
    let d2 = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    let step = &d2["schema"]["properties"]["steps"]["items"];
    assert_eq!(step["required"], serde_json::json!(["candidate", "value"]));

    let d1 = require_python!(explain("qwen_interpreter.py", &selection("d1")));
    let step = &d1["schema"]["properties"]["steps"]["items"];
    assert_eq!(step["required"], serde_json::json!(["candidate"]));
    assert!(
        step["properties"]["value"].is_null(),
        "d1 must not offer a value, got {step}"
    );
}

/// The backend uses the contract it was handed rather than one of its own.
///
/// This is the property that makes the two copies one. If the backend ever
/// composes its own schema again, what it reports here stops matching what the
/// engine sent, and this fails.
#[test]
fn the_schema_the_engine_sent_is_the_one_used() {
    let request = utterance("half of last time");
    let sent: serde_json::Value = serde_json::from_str(&request).expect("the fixture is JSON");
    let plan = require_python!(explain("qwen_interpreter.py", &request));

    assert_eq!(
        plan["schema"], sent["schema"],
        "the backend must compile the schema the engine sent, not one of its own"
    );
}

/// A request carrying no contract is refused rather than served from a guess.
///
/// A backend that invents a contract when none arrived is how the copies came
/// to differ in the first place.
#[test]
fn a_request_without_a_contract_is_refused() {
    let bare = serde_json::json!({"utterance": "make me an espresso"}).to_string();
    let python = require_python!(python());

    let mut child = Command::new(python)
        .arg(sdk().join("examples/interpreters/qwen_interpreter.py"))
        .arg("--explain")
        .current_dir(sdk())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the backend should start");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{bare}\n").as_bytes())
        .unwrap();

    let out = child.wait_with_output().expect("should finish");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("schema"),
        "the refusal should name what was missing, got: {stderr}"
    );
}

/// The goals and the candidates have to reach the model, or it is choosing
/// from nothing.
#[test]
fn the_prompt_carries_the_goal_and_every_candidate() {
    let plan = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    let prompt = plan["prompt"].as_str().expect("a prompt");

    assert!(prompt.contains("shot_time"), "{prompt}");
    assert!(prompt.contains("adi.espresso.grind.set_setting"), "{prompt}");
    assert!(prompt.contains("adi.espresso.dose.set_mass"), "{prompt}");
    assert!(
        prompt.contains("-1.4"),
        "per_unit is how a correction is sized: {prompt}"
    );
    assert!(
        prompt.contains("24"),
        "current is what d2 adds the change to: {prompt}"
    );
}

/// A field the engine troubles to send and the prompt discards is invisible
/// from both ends: the record shows it was supplied and the model never saw
/// it. The first version of this backend dropped four.
#[test]
fn nothing_the_engine_sends_is_dropped() {
    let plan = require_python!(explain("qwen_interpreter.py", &selection("d2")));
    let prompt = plan["prompt"].as_str().expect("a prompt");

    // ADI-P 3.3.1: a selector at D1 or D2 must be given the history the engine
    // holds, or it can disagree with the ranking but never on evidence.
    assert!(prompt.contains("cycle-0012"), "the remembered cycles: {prompt}");

    // The diagnosis the correction answers. Two goals on one variable can want
    // opposite things for different reasons.
    assert!(prompt.contains("under_extracted"), "the goal's cause: {prompt}");

    // At d2 the value sets the parameter, so which parameter and in what unit
    // is what the number means.
    assert!(prompt.contains("setting"), "the parameter: {prompt}");
    assert!(prompt.contains("step"), "the parameter's unit: {prompt}");
}

/// The same rule on the other seam, which is where it was not being kept.
///
/// This checked only the selection path, and the interpreter path was dropping
/// three fields: the machine identity, the notes and the decision history. Two
/// of the three are what an answer is composed from, so the omission and the
/// missing answer arm hid each other.
#[test]
fn nothing_the_engine_sends_on_the_spoken_path_is_dropped() {
    let plan = require_python!(explain("qwen_interpreter.py", &utterance("why was it sour")));

    // The context goes in the system message on this seam, so both halves of
    // what the model is sent are checked.
    let prompt = format!(
        "{}\n{}",
        plan["instruction"].as_str().expect("an instruction"),
        plan["prompt"].as_str().expect("a prompt")
    );
    let prompt = prompt.as_str();

    // ADI-P 9.4.5: what the machine is. "What model are you" is answerable
    // from nothing else.
    assert!(
        prompt.contains("EX-900"),
        "the machine identity: {prompt}"
    );

    // The note text, and the identifier an answer cites in drew_on.
    assert!(
        prompt.contains("under-extraction"),
        "the note's text: {prompt}"
    );
    assert!(
        prompt.contains("adi.espresso.note.sour_vs_bitter"),
        "the note's id, which an answer has to name: {prompt}"
    );

    // ADI-P 3.3.1 on this seam too.
    assert!(
        prompt.contains("cycle-0012"),
        "the decisions already taken: {prompt}"
    );

    // And what was already there, so this does not pass by replacing it.
    assert!(prompt.contains("shot_time"), "the variables: {prompt}");
}

/// The switch that stops Qwen reasoning aloud belongs in exactly one place.
///
/// It was briefly added twice, once where the prompt is built and once where
/// it is sent. Harmless, since a model that does not recognise it reads it as
/// ordinary text, and still wrong to ship.
#[test]
fn the_direct_answer_switch_is_not_repeated() {
    for request in [utterance("half of last time"), selection("d2")] {
        let plan = require_python!(explain("qwen_interpreter.py", &request));
        let prompt = plan["prompt"].as_str().expect("a prompt");
        assert_eq!(
            prompt.matches("/no_think").count(),
            0,
            "the prompt is built bare and the switch is appended once when sent: {prompt}"
        );
    }
}

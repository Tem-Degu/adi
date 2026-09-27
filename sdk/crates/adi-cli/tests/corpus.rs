//! The conformance corpus must pass on every build.
//!
//! Running it through the binary rather than calling the module directly is
//! deliberate: this is the same command a manufacturer runs, so the test
//! covers the tool as shipped rather than an internal entry point that could
//! drift from it.

use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_adi"))
        .args(args)
        .output()
        .expect("the adi binary should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn the_espresso_corpus_passes() {
    let (ok, text) = run(&["conformance", "../../conformance/espresso/cases"]);
    assert!(ok, "conformance failed:\n{text}");
    assert!(
        text.contains("0 failed"),
        "expected no failures:\n{text}"
    );
}

/// Pending cases are reported and do not fail the run. They are declared gaps
/// rather than regressions, and a runner that hid them would be worse than one
/// that either passed or failed them.
#[test]
fn pending_cases_are_reported_and_do_not_fail_the_run() {
    let (ok, text) = run(&["conformance", "../../conformance/espresso/cases"]);
    assert!(ok);
    assert!(text.contains("PEND"), "a pending case should be visible:\n{text}");
    assert!(
        text.contains("interpreter"),
        "the reason should name what is missing:\n{text}"
    );
}

/// A request expressed as structured intent runs end to end. Only turning words
/// into that structure needs a model, so the corpus carries the same request
/// twice: once as intent, which passes, and once as speech, which is pending.
#[test]
fn the_request_path_runs_without_a_model() {
    let (ok, text) = run(&["conformance", "../../conformance/espresso/cases"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("PASS  less-sugar-never-touches-extraction"),
        "the intent form should run:\n{text}"
    );
    assert!(
        text.contains("PEND  spoken-request-needs-a-model"),
        "the speech form should be declared as pending:\n{text}"
    );
}

/// A corpus path that does not exist must fail loudly rather than reporting a
/// vacuous pass. A conformance tool that succeeds on an empty run is worse
/// than no tool.
#[test]
fn a_missing_corpus_is_an_error() {
    let (ok, _) = run(&["conformance", "../../conformance/does-not-exist"]);
    assert!(!ok, "a missing corpus must not report success");
}

#[test]
fn the_shipped_examples_validate() {
    let (ok, text) = run(&[
        "validate",
        "../../examples/declarations/ex900.yaml",
        "--skill",
        "../../examples/declarations/ex900.skill.yaml",
    ]);
    assert!(ok, "ex900 should validate:\n{text}");

    let (ok, text) = run(&["validate", "../../examples/declarations/neutral-machine.yaml"]);
    assert!(ok, "the neutral fixture should validate:\n{text}");
}

/// The M1 gate. With an interpreter supplied, spoken words run the whole way
/// to a selected tool and nothing is left pending.
///
/// The interpreter is scripted rather than a model. What this proves is that
/// the path is complete, so that when a model is substituted a failure is
/// attributable to the model rather than to everything around it.
#[test]
fn a_spoken_request_runs_end_to_end_with_an_interpreter() {
    let (ok, text) = run(&[
        "conformance",
        "../../conformance/espresso/cases",
        "--interpreter",
        "../../conformance/espresso/interpreter.yaml",
    ]);

    assert!(ok, "the corpus should pass: {text}");
    assert!(text.contains("0 pending"), "nothing pending: {text}");
    assert!(!text.contains("FAIL"), "and nothing failing: {text}");
}

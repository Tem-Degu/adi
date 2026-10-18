//! A machine runs on the profile it declares, including one with no domain.
//!
//! The engine has always taken a profile as a runtime plug-in. This tool
//! ignored that and loaded espresso whatever the machine said, which left the
//! whole SDK usable only for coffee while the architecture underneath was
//! general. These tests hold it to the declaration.
//!
//! Run through the binary rather than the module, so what is covered is the
//! command a developer actually types.

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

const PRINTER: &str = "../../examples/declarations/printer.yaml";
const PRINTER_SKILL: &str = "../../examples/declarations/printer.skill.yaml";
const PRINTER_KNOWLEDGE: &str = "../../examples/declarations/printer.knowledge.yaml";

/// The claim the repository makes, demonstrated on a machine that is not a
/// coffee machine: describe it in two files, declare the base profile, and the
/// engine reasons about it. No Rust, and no profile written for printing.
#[test]
fn a_machine_that_is_not_espresso_validates_on_the_base_profile() {
    let (ok, text) = run(&[
        "validate",
        PRINTER,
        "--skill",
        PRINTER_SKILL,
        "--knowledge",
        PRINTER_KNOWLEDGE,
    ]);
    assert!(ok, "the printer example should validate:\n{text}");
    assert!(text.contains("base/v1"), "the declared profile should be reported:\n{text}");
    assert!(text.contains("knowledge: ok"), "its knowledge should layer:\n{text}");
}

/// The outcomes come entirely from the manufacturer's own file. Without this
/// the planner ignores every skill response, so the tools are callable and
/// never selectable, and nothing says why.
#[test]
fn the_manufacturers_own_outcomes_are_what_the_engine_can_move() {
    let (ok, text) = run(&[
        "capabilities",
        PRINTER,
        "--skill",
        PRINTER_SKILL,
        "--knowledge",
        PRINTER_KNOWLEDGE,
    ]);
    assert!(ok, "{text}");
    assert!(text.contains("x.inkworks.ink_coverage"), "{text}");
    assert!(text.contains("x.inkworks.dry_time"), "{text}");
    assert!(
        !text.contains("not modelled"),
        "every outcome the skill names was declared, so none should be ignored:\n{text}"
    );
}

/// Espresso is not special. It is selected the same way, by name.
#[test]
fn espresso_is_selected_the_same_way() {
    let (ok, text) = run(&[
        "validate",
        "../../examples/declarations/ex900.yaml",
        "--skill",
        "../../examples/declarations/ex900.skill.yaml",
    ]);
    assert!(ok, "{text}");
    assert!(text.contains("espresso/v1"), "{text}");
}

/// A declaration naming a profile this build does not carry is still a valid
/// declaration. A validator that failed it would be reporting on itself rather
/// than on the document, so it says what it could not check and carries on.
#[test]
fn an_unknown_profile_is_reported_and_does_not_fail_validation() {
    let (ok, text) = run(&["validate", "../../examples/declarations/neutral-machine.yaml"]);
    assert!(
        ok,
        "an unavailable profile is a property of this build, not a fault in the document:\n{text}"
    );
    assert!(text.contains("not in this build"), "{text}");
    assert!(
        text.contains("base/v1"),
        "the message should point at the profile that needs no code:\n{text}"
    );
}

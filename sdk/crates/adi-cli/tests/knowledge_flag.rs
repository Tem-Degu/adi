//! ADI-P 6: loading a manufacturer's own knowledge.
//!
//! The mechanism was complete and tested from the start, and nothing called
//! it. `add_extension` appeared only in its own crate's tests, so a
//! manufacturer following the README's instruction to declare an outcome or a
//! note wrote a file no shipped command could read. These exist so that stays
//! fixed.

use std::path::Path;
use std::process::Command;

fn sdk() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn adi(args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_adi"))
        .args(args)
        .current_dir(sdk())
        .output()
        .expect("the adi binary should start");

    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

const DECLARATION: &str = "examples/declarations/ex900.yaml";
const SKILL: &str = "examples/declarations/ex900.skill.yaml";
const KNOWLEDGE: &str = "examples/declarations/ex900.knowledge.yaml";

#[test]
fn a_knowledge_extension_loads() {
    let (out, ok) = adi(&["validate", DECLARATION, "--skill", SKILL, "--knowledge", KNOWLEDGE]);
    assert!(ok, "{out}");
    assert!(out.contains("knowledge: ok"), "{out}");
}

/// Without the flag nothing changes, which is what every machine that has no
/// extension relies on.
#[test]
fn no_flag_means_base_knowledge_and_no_mention_of_it() {
    let (out, ok) = adi(&["validate", DECLARATION, "--skill", SKILL]);
    assert!(ok, "{out}");
    assert!(!out.contains("knowledge"), "{out}");
}

/// ADI-P 6.5: a failing extension is rejected with a reason and the engine
/// continues on the knowledge it already has. A bad extension degrades a
/// machine to standard behaviour; it never stops it working.
#[test]
fn a_bad_extension_is_rejected_and_the_command_still_succeeds() {
    let dir = std::env::temp_dir().join("adi_knowledge_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("widens.yaml");

    // 6.4 forbids widening a constraint the base layer set.
    std::fs::write(
        &path,
        "namespace: \"x.acme\"\n\
         extension_version: \"1.0\"\n\
         extends: { domain_profile: \"espresso/v1\", base_min_version: \"1.0\" }\n\
         constraints:\n\
         \x20 brew_temperature: { min: 80.0, max: 110.0, unit: c }\n",
    )
    .unwrap();

    let (out, ok) = adi(&[
        "validate",
        DECLARATION,
        "--skill",
        SKILL,
        "--knowledge",
        path.to_str().unwrap(),
    ]);

    assert!(out.contains("knowledge: REJECTED"), "{out}");
    assert!(
        ok,
        "the command must still succeed on base knowledge, per 6.5:\n{out}"
    );
    assert!(out.contains("declaration: ok"), "{out}");
}

/// The point of loading one: a vendor variable becomes an outcome the engine
/// models, which is what 9.4.1 requires before a request can name it.
#[test]
fn a_vendor_variable_becomes_modelled() {
    let (without, _) = adi(&["capabilities", DECLARATION, "--skill", SKILL]);
    let (with, ok) = adi(&[
        "capabilities", DECLARATION, "--skill", SKILL, "--knowledge", KNOWLEDGE,
    ]);
    assert!(ok, "{with}");

    assert!(!without.contains("knowledge: ok"), "{without}");
    assert!(with.contains("knowledge: ok"), "{with}");
}

/// A skill response naming a variable nothing models is skipped by the planner
/// rather than refused, so the tool it describes is never chosen and nothing
/// says why. `capabilities` now says why.
#[test]
fn a_response_naming_an_unmodelled_variable_is_reported() {
    let dir = std::env::temp_dir().join("adi_knowledge_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stray.skill.yaml");

    let mut skill: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(sdk().join(SKILL)).unwrap()).unwrap();
    let responses = skill["responses"].as_sequence_mut().unwrap();
    let mut stray = responses[0].clone();
    stray["variable"] = serde_yaml::Value::String("x.acme.tray_state".into());
    responses.push(stray);
    std::fs::write(&path, serde_yaml::to_string(&skill).unwrap()).unwrap();

    let (out, ok) = adi(&[
        "capabilities",
        DECLARATION,
        "--skill",
        path.to_str().unwrap(),
    ]);
    assert!(ok, "{out}");
    assert!(
        out.contains("not modelled, so ignored"),
        "an unmodelled variable must be called out:\n{out}"
    );
    assert!(out.contains("x.acme.tray_state"), "{out}");

    // And loading the extension that declares it moves it to the usable list.
    let (with, ok) = adi(&[
        "capabilities",
        DECLARATION,
        "--skill",
        path.to_str().unwrap(),
        "--knowledge",
        KNOWLEDGE,
    ]);
    assert!(ok, "{with}");
    assert!(
        !with.contains("not modelled, so ignored"),
        "declaring it should make it modelled:\n{with}"
    );
}

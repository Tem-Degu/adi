//! Knowledge base and extension shapes.
//!
//! The gate for this step: a base file and a vendor extension both parse, and
//! both collapse to the same layer type, so resolution has one kind of thing to
//! combine rather than a special case per source.
//!
//! Fixtures are a fictional domain. This crate defines the format; if it could
//! only be exercised against espresso, "domain-neutral" would be a claim rather
//! than a property. The espresso base and the example from PROTOCOL.md 6.2 are
//! tested in `adi-domain-espresso`, where that vocabulary belongs.

use adi_protocol::knowledge::ExtensionError;
use adi_protocol::{KnowledgeBase, KnowledgeExtension, Layer, Range, Unit};

fn load<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/knowledge")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_yaml::from_str(&text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn base() -> KnowledgeBase {
    load("neutral-base.yaml")
}

fn extension() -> KnowledgeExtension {
    load("neutral-extension.yaml")
}

// ------------------------------------------------------------------ shapes

#[test]
fn a_base_parses() {
    let b = base();
    assert_eq!(b.profile, "example/v1");
    assert_eq!(b.styles.len(), 2);
    assert_eq!(b.material_priors.len(), 2);
    assert!(!b.constraints.is_empty(), "the base owns hard constraints");
    assert!(
        !b.preference_variables.is_empty(),
        "the base owns the preference variable list"
    );

    // Default steps carry units, like every other quantity in the protocol.
    let step = b.default_steps.get("cycle_time").expect("a default step");
    assert_eq!(step.unit, Unit::S);
    assert_eq!(step.value, 2.0);
}

#[test]
fn an_extension_parses() {
    let e = extension();
    assert_eq!(e.namespace, "x.acme");
    assert_eq!(e.styles.len(), 1);
    assert_eq!(e.material_priors.len(), 1);
    assert_eq!(e.defaults.len(), 1);
    assert_eq!(e.constraints.len(), 1, "an extension may narrow a constraint");
}

/// The gate for step 1. Two documents of different shape, one thing to resolve.
#[test]
fn both_collapse_to_the_same_layer_type() {
    let b = base().into_layer();
    let v = extension().into_layer(Layer::Vendor);

    assert_eq!(b.layer, Layer::Base);
    assert_eq!(v.layer, Layer::Vendor);
    assert!(b.namespace.is_none(), "the base owns the bare namespace");
    assert_eq!(v.namespace.as_deref(), Some("x.acme"));

    // The extension's `defaults` list became the same map the base uses, so
    // resolution never has to know which shape it came from.
    assert_eq!(v.default_steps.get("input_mass").map(|q| q.value), Some(9.0));
    assert_eq!(
        v.default_steps.get("input_mass").map(|q| q.unit),
        Some(Unit::G)
    );
    assert_eq!(b.default_steps.get("cycle_time").map(|q| q.value), Some(2.0));
}

/// Only the base may say which variables express preference. A vendor able to
/// add one could reclassify a process variable as taste and put it beyond the
/// engine's reach.
#[test]
fn an_extension_cannot_declare_preference_variables() {
    assert!(extension().into_layer(Layer::Vendor).preference_variables.is_empty());
    assert!(!base().into_layer().preference_variables.is_empty());
}

// -------------------------------------------------------------- validation

#[test]
fn a_well_formed_extension_validates() {
    extension()
        .validate(&base())
        .expect("the shipped example must validate against the base it extends");
}

#[test]
fn an_unnamespaced_identifier_is_refused() {
    let mut e = extension();
    e.styles[0].id = "custom".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::NotNamespaced { .. })
    ));
}

/// Redefinition is checked before namespacing, because it is the more specific
/// complaint and the more useful message.
#[test]
fn redefining_a_base_identifier_is_refused() {
    let mut e = extension();
    e.styles[0].id = "adi.example.style.standard".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::Redefinition(_))
    ));
}

#[test]
fn inheriting_from_nothing_is_refused() {
    let mut e = extension();
    e.styles[0].inherits = Some("adi.example.style.nonexistent".to_string());
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::UnknownParent { .. })
    ));
}

/// A cycle must be reported rather than overflowing the stack, so the walk is
/// iterative with a budget.
#[test]
fn an_inheritance_cycle_is_refused() {
    let mut e = extension();
    let first = e.styles[0].clone();
    let mut second = first.clone();
    second.id = "x.acme.style.other".to_string();
    second.inherits = Some(first.id.clone());
    e.styles[0].inherits = Some(second.id.clone());
    e.styles.push(second);

    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::InheritanceCycle(_))
    ));
}

/// An empty range makes every value illegal, which is never what anyone meant
/// to write.
#[test]
fn an_empty_range_is_refused() {
    let mut e = extension();
    e.styles[0].targets.insert(
        "cycle_time".to_string(),
        Range {
            min: 10.0,
            max: 1.0,
            resolution: None,
            unit: Unit::S,
        },
    );
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::EmptyRange(_))
    ));
}

#[test]
fn extending_the_wrong_profile_is_refused() {
    let mut e = extension();
    e.extends.domain_profile = "somewhere/v9".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::WrongProfile { .. })
    ));
}

/// Inheriting from a base style is the normal case and must keep working, since
/// it is how a vendor starts from something rather than restating it.
#[test]
fn inheriting_from_the_base_is_allowed() {
    let e = extension();
    assert_eq!(
        e.styles[0].inherits.as_deref(),
        Some("adi.example.style.standard")
    );
    e.validate(&base()).expect("inheriting from the base is the point");
}

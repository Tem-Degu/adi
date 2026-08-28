//! The shipped espresso knowledge base, and the example from PROTOCOL.md 6.2.
//!
//! These live here rather than in `adi-protocol` because they are written in
//! espresso vocabulary. The format itself is tested against a fictional domain
//! in that crate, so neutrality stays demonstrated rather than asserted.

use adi_protocol::{Knowledge, KnowledgeBase, KnowledgeExtension, Layer, Unit};

fn base() -> KnowledgeBase {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../profiles/espresso/knowledge.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_yaml::from_str(&text).expect("the espresso knowledge base should parse")
}

/// Lifted verbatim from PROTOCOL.md 6.2.
///
/// Copied rather than paraphrased on purpose: if the specification and the
/// types drift apart, this fails. A published example that cannot parse is
/// worse than no example, because a manufacturer will type it in and it will
/// not work.
const SPEC_EXAMPLE: &str = r#"
namespace: "x.example"
extension_version: "1.4"
extends: { domain_profile: "espresso/v1", base_min_version: "1.0" }

styles:
  - id: "x.example.style.flat_white"
    inherits: "adi.espresso.style.espresso"
    targets:
      ratio:       { min: 1.8, max: 2.2, unit: none }
      shot_time:   { min: 25, max: 32, unit: s }
      milk_volume: { min: 90, max: 130, unit: ml }

material_priors:
  - id: "x.example.material.house_blend"
    attributes: { roast: "medium_dark" }
    offsets:    { grind_setting: -3.0 }
    targets:
      brew_temperature: { min: 90.0, max: 92.0, unit: c }

defaults:
  - variable: "dose_mass"
    value: 9.0
    unit: g
    rationale_note_id: "brew_unit_basket_size"

constraints:
  brew_temperature: { min: 88.0, max: 96.0, unit: c }
"#;

#[test]
fn the_shipped_knowledge_base_parses() {
    let b = base();
    assert_eq!(b.profile, "espresso/v1");

    // The styles the espresso profile actually reasons about.
    for id in [
        "adi.espresso.style.espresso",
        "adi.espresso.style.ristretto",
        "adi.espresso.style.lungo",
        "adi.espresso.style.for_milk",
    ] {
        assert!(
            b.styles.iter().any(|s| s.id == id),
            "the base should declare '{id}'"
        );
    }

    // Roast priors, with the grind offset that starts a new bag near the mark.
    let light = b
        .material_priors
        .iter()
        .find(|m| m.id == "adi.espresso.roast.light")
        .expect("a light roast prior");
    assert_eq!(light.offsets.get("grind_setting"), Some(&-3.0));

    let step = b.default_steps.get("shot_time").expect("a shot_time step");
    assert_eq!(step.unit, Unit::S);
    assert!(!b.constraints.is_empty());
    assert!(b
        .preference_variables
        .iter()
        .any(|v| v.as_str() == "milk_volume"));
}

/// Ristretto inherits from espresso, so the chain has to resolve.
#[test]
fn styles_inherit_within_the_base() {
    let b = base();
    let ristretto = b
        .styles
        .iter()
        .find(|s| s.id == "adi.espresso.style.ristretto")
        .expect("ristretto");
    assert_eq!(
        ristretto.inherits.as_deref(),
        Some("adi.espresso.style.espresso")
    );
}

#[test]
fn the_spec_example_parses_and_validates() {
    let e: KnowledgeExtension =
        serde_yaml::from_str(SPEC_EXAMPLE).expect("PROTOCOL.md 6.2 should parse");
    e.validate(&base())
        .expect("PROTOCOL.md 6.2 should validate against the shipped base");

    let layer = e.into_layer(Layer::Vendor);
    assert_eq!(layer.namespace.as_deref(), Some("x.example"));
    assert_eq!(layer.default_steps.get("dose_mass").map(|q| q.value), Some(9.0));
}

// ------------------------------------------------------ the wired profile

/// The profile resolves from the compiled-in knowledge base, so editing
/// `profiles/espresso/knowledge.yaml` changes what the engine decides. Before
/// this, targets came from a table inside `base_targets` that duplicated the
/// file and had already drifted from it.
#[test]
fn the_profile_resolves_from_the_knowledge_file() {
    use adi_core::domain::Domain;
    let d = adi_domain_espresso::Espresso::new();

    let file = base();
    let shipped = file
        .styles
        .iter()
        .find(|s| s.id == "adi.espresso.style.lungo")
        .unwrap()
        .targets["shot_time"]
        .clone();

    let resolved = d
        .knowledge()
        .resolve("adi.espresso.style.lungo", None)
        .unwrap();
    let got = &resolved.targets.get("shot_time").unwrap().value;
    assert_eq!((got.min, got.max), (shipped.min, shipped.max));
}

/// The old table matched on short names and returned an espresso for anything
/// it did not recognise, so a typo produced a plausible wrong drink instead of
/// an error.
#[test]
fn an_unknown_style_is_refused_rather_than_defaulted() {
    use adi_core::domain::Domain;
    let d = adi_domain_espresso::Espresso::new();
    assert!(d.knowledge().resolve("espresso", None).is_err());
    assert!(d.knowledge().resolve("cortado", None).is_err());
}

#[test]
fn a_vendor_extension_adds_a_style_to_the_profile() {
    use adi_core::domain::Domain;
    let mut d = adi_domain_espresso::Espresso::new();
    let e: KnowledgeExtension = serde_yaml::from_str(SPEC_EXAMPLE).unwrap();
    d.add_extension(e, Layer::Vendor).expect("a valid extension");

    let r = d
        .knowledge()
        .resolve("x.example.style.flat_white", None)
        .unwrap();
    assert_eq!(r.targets.get("milk_volume").unwrap().layer, Layer::Vendor);
}

/// ADI-P 6.5: a failing extension is rejected with a reason and the machine
/// continues on the knowledge it already has.
#[test]
fn a_bad_extension_leaves_the_profile_working() {
    use adi_core::domain::Domain;
    let mut d = adi_domain_espresso::Espresso::new();
    let mut e: KnowledgeExtension = serde_yaml::from_str(SPEC_EXAMPLE).unwrap();
    // Reaches above the base ceiling of 98, which 6.4 forbids.
    e.constraints.insert(
        "brew_temperature".into(),
        adi_protocol::Range {
            min: 80.0,
            max: 110.0,
            resolution: None,
            unit: Unit::C,
        },
    );

    assert!(d.add_extension(e, Layer::Vendor).is_err());
    d.knowledge()
        .resolve("adi.espresso.style.espresso", None)
        .expect("the machine must keep working on base knowledge");
    assert!(d
        .knowledge()
        .resolve("x.example.style.flat_white", None)
        .is_err());
}

// ------------------------------------------------------------- resolution

fn knowledge() -> Knowledge {
    Knowledge::new(base().into_layer()).expect("the espresso base")
}

fn with_vendor() -> Knowledge {
    let mut k = knowledge();
    let e: KnowledgeExtension = serde_yaml::from_str(SPEC_EXAMPLE).unwrap();
    k.add(e.into_layer(Layer::Vendor)).expect("the spec example");
    k
}

/// Every shipped style must resolve. A style that cannot resolve is a drink the
/// machine would refuse to make.
#[test]
fn every_shipped_style_resolves() {
    let k = knowledge();
    for id in k.style_ids() {
        k.resolve(id, None)
            .unwrap_or_else(|e| panic!("style '{id}' does not resolve: {e}"));
    }
}

/// The case that decided the design. `lungo` inherits from `espresso` and sets
/// a ratio of 3.0 to 4.0 against the parent's 1.8 to 2.4. The ranges are
/// disjoint. Had inheritance intersected rather than overridden, resolution
/// would have produced an empty range and every lungo would have failed.
#[test]
fn lungo_overrides_a_ratio_disjoint_from_its_parent() {
    let k = knowledge();
    let espresso = k.resolve("adi.espresso.style.espresso", None).unwrap();
    let lungo = k.resolve("adi.espresso.style.lungo", None).unwrap();

    let e = &espresso.targets.get("ratio").unwrap().value;
    let l = &lungo.targets.get("ratio").unwrap().value;

    assert!(
        l.min > e.max,
        "the fixture only tests what it claims if these stay disjoint"
    );
    assert_eq!((l.min, l.max), (3.0, 4.0));
    assert!(l.min < l.max, "resolution must not collapse the range");
}

/// Brew temperature is not named by any child style, so it comes down from
/// espresso unchanged.
#[test]
fn a_child_inherits_what_it_does_not_name() {
    let k = knowledge();
    let parent = k.resolve("adi.espresso.style.espresso", None).unwrap();
    let child = k.resolve("adi.espresso.style.ristretto", None).unwrap();

    let p = &parent.targets.get("brew_temperature").unwrap().value;
    let c = &child.targets.get("brew_temperature").unwrap().value;
    assert_eq!((p.min, p.max), (c.min, c.max));
}

/// A roast prior moves the starting grind so a new bag lands near the mark
/// rather than at the last bag's setting.
#[test]
fn a_roast_prior_supplies_a_grind_offset() {
    let r = knowledge()
        .resolve(
            "adi.espresso.style.espresso",
            Some("adi.espresso.roast.light"),
        )
        .unwrap();
    assert_eq!(r.offsets.get("grind_setting").map(|a| a.value), Some(-3.0));
}

/// The dark roast wants 88 to 92 while the espresso style says 90 to 96. The
/// prior reaches below the style, which is ordinary: dark roasts are brewed
/// cooler. Resolution takes the overlap, so the shot is still an espresso and
/// still runs cool for the roast.
#[test]
fn a_prior_leaning_past_the_style_resolves_to_the_overlap() {
    let r = knowledge()
        .resolve(
            "adi.espresso.style.espresso",
            Some("adi.espresso.roast.dark"),
        )
        .unwrap();
    let t = &r.targets.get("brew_temperature").unwrap().value;
    assert_eq!((t.min, t.max), (90.0, 92.0));
}

/// A prior that sits inside the style simply narrows it.
#[test]
fn a_prior_inside_the_style_narrows_it() {
    let r = knowledge()
        .resolve(
            "adi.espresso.style.espresso",
            Some("adi.espresso.roast.light"),
        )
        .unwrap();
    let t = &r.targets.get("brew_temperature").unwrap().value;
    assert_eq!((t.min, t.max), (92.0, 96.0));
}

/// Roast changes how the coffee behaves, not what the drink is. Ratio separates
/// a ristretto from a lungo, so no roast prior may touch it.
///
/// An earlier version of the knowledge did, with ranges written for a plain
/// espresso, and a lungo from light roast beans became unresolvable.
#[test]
fn no_roast_prior_constrains_ratio() {
    let b = base();
    for prior in &b.material_priors {
        assert!(
            !prior.targets.contains_key("ratio"),
            "'{}' sets a ratio, which belongs to the style",
            prior.id
        );
    }
}

/// Every style and roast the profile ships must combine. A pair that does not
/// resolve is a drink the machine would refuse to make from a bag it accepts.
#[test]
fn every_style_and_roast_pair_resolves() {
    let k = knowledge();
    let roasts = [
        "adi.espresso.roast.light",
        "adi.espresso.roast.medium",
        "adi.espresso.roast.dark",
    ];
    for style in k.style_ids() {
        for roast in roasts {
            let r = k
                .resolve(style, Some(roast))
                .unwrap_or_else(|e| panic!("{style} with {roast}: {e}"));
            for (key, t) in r.targets.iter() {
                assert!(
                    t.value.min <= t.value.max,
                    "{style} with {roast}: '{key}' resolved to an empty range"
                );
            }
        }
    }
}

/// The vendor style from PROTOCOL.md 6.2 resolves through the base it inherits
/// from, and its milk volume survives even though the base knows no such target.
#[test]
fn the_spec_vendor_style_resolves_through_the_base() {
    let k = with_vendor();
    let r = k.resolve("x.example.style.flat_white", None).unwrap();

    let milk = r.targets.get("milk_volume").expect("milk_volume");
    assert_eq!((milk.value.min, milk.value.max), (90.0, 130.0));
    assert_eq!(milk.layer, Layer::Vendor);

    // brew_temperature is named by neither the vendor style nor inherited from
    // anywhere but the base espresso style, so the base keeps the credit.
    assert_eq!(
        r.targets.get("brew_temperature").unwrap().layer,
        Layer::Base
    );
}

/// Milk volume is taste, not process. The engine must never try to fix a sour
/// shot by pouring more milk.
#[test]
fn milk_volume_is_a_preference_and_grind_is_not() {
    let k = with_vendor();
    assert!(k.is_preference("milk_volume"));
    assert!(!k.is_preference("grind_setting"));
}

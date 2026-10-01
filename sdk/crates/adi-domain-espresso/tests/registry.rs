//! The espresso registry.

/// PROTOCOL.md 4.2: a profile MUST supply a description for every tool in its
/// registry, so a manufacturer declaring one does not have to write it and a
/// model is never offered a registry tool it cannot identify.
///
/// Reads the identifiers from the module rather than listing them, so a tool
/// added without a description fails here rather than reaching a model
/// undescribed.
#[test]
fn every_registry_tool_has_a_description() {
    use adi_domain_espresso::registry::{description, tools};

    const ALL: &[(&str, &str)] = &[
        ("GRIND_SET", tools::GRIND_SET),
        ("GRIND_PURGE", tools::GRIND_PURGE),
        ("DOSE_BY_MASS", tools::DOSE_BY_MASS),
        ("TAMP", tools::TAMP),
        ("BREW_SET_TEMP", tools::BREW_SET_TEMP),
        ("BREW_LOAD_PROFILE", tools::BREW_LOAD_PROFILE),
        ("BREW_START", tools::BREW_START),
        ("BREW_STOP", tools::BREW_STOP),
        ("BREW_SET_LIVE_TARGET", tools::BREW_SET_LIVE_TARGET),
        ("PUCK_EJECT", tools::PUCK_EJECT),
        ("ADDITIVE_DISPENSE", tools::ADDITIVE_DISPENSE),
        ("MAINTENANCE_FLUSH", tools::MAINTENANCE_FLUSH),
    ];

    for (name, id) in ALL {
        let text = description(id)
            .unwrap_or_else(|| panic!("{name} ({id}) has no description"));

        // Non-empty, and not merely the identifier restated. A length threshold
        // would be a rule invented here rather than one that matters:
        // "Begins extraction." is complete, and brew.start competes with
        // nothing, so there is nothing to tell apart.
        assert!(!text.trim().is_empty(), "{name} has an empty description");
        assert!(
            !text.contains(id),
            "{name} repeats its identifier instead of saying what it does: {text:?}"
        );
    }
}

/// A vendor identifier has no entry, which is why a vendor tool must supply its
/// own and validation refuses one that does not.
#[test]
fn a_vendor_tool_has_no_profile_description() {
    use adi_domain_espresso::registry::description;

    assert!(description("x.acme.aux.actuate_3").is_none());
}

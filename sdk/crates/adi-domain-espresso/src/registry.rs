//! The espresso semantic registry. ADI-P section 7.
//!
//! These identifiers fix meaning across manufacturers. A machine declaring
//! `GRIND_SET` asserts the tool adjusts grind fineness and nothing else.

pub const PROFILE_ID: &str = "espresso/v1";

pub mod tools {
    pub const GRIND_SET: &str = "adi.espresso.grind.set_setting";
    pub const GRIND_PURGE: &str = "adi.espresso.grind.purge";
    pub const DOSE_BY_MASS: &str = "adi.espresso.dose.by_mass";
    pub const TAMP: &str = "adi.espresso.tamp";
    pub const BREW_SET_TEMP: &str = "adi.espresso.brew.set_temperature";
    pub const BREW_LOAD_PROFILE: &str = "adi.espresso.brew.load_profile";
    pub const BREW_START: &str = "adi.espresso.brew.start";
    pub const BREW_STOP: &str = "adi.espresso.brew.stop";
    pub const BREW_SET_LIVE_TARGET: &str = "adi.espresso.brew.set_live_target";
    pub const PUCK_EJECT: &str = "adi.espresso.puck.eject";
    pub const ADDITIVE_DISPENSE: &str = "adi.espresso.additive.dispense";
    pub const MAINTENANCE_FLUSH: &str = "adi.espresso.maintenance.flush";
}

pub mod signals {
    pub const MASS: &str = "adi.espresso.signal.mass";
    pub const FLOW: &str = "adi.espresso.signal.flow";
    pub const PRESSURE: &str = "adi.espresso.signal.pressure";
    pub const GROUP_TEMP: &str = "adi.espresso.signal.group_temp";
    pub const ELAPSED: &str = "adi.espresso.signal.elapsed";
    pub const GRIND_SETTING: &str = "adi.espresso.signal.grind_setting";
    pub const DOSE_MASS: &str = "adi.espresso.signal.dose_mass";
}

/// What each registry tool is for, in words.
///
/// From the PROTOCOL.md 7.1 table, which was prose no code read. Supplied by the
/// profile rather than retyped by every manufacturer: a registry identifier
/// means the same thing on every machine that declares it, so the description
/// belongs with the identifier and not with one vendor's copy of it.
///
/// A manufacturer MAY still write their own on the tool, which takes precedence,
/// because what is true of their mechanism is theirs to state. A vendor tool has
/// no entry here and must carry its own.
pub fn description(tool: &str) -> Option<&'static str> {
    Some(match tool {
        tools::GRIND_SET => {
            "Sets grind fineness. Finer slows the flow and lengthens extraction.              The primary control over extraction rate."
        }
        tools::GRIND_PURGE => {
            "Discards grounds retained in the grinder from a previous setting.              Worth doing after a large change, since retained grounds are at the              old setting."
        }
        tools::DOSE_BY_MASS => {
            "Dispenses ground coffee by mass. Changes the strength of the result              as well as the extraction rate."
        }
        tools::TAMP => "Compresses the puck to an even bed before extraction.",
        tools::BREW_SET_TEMP => {
            "Sets brew water temperature. Higher extracts more, towards bitter;              lower extracts less, towards sour."
        }
        tools::BREW_LOAD_PROFILE => {
            "Installs a pressure or flow profile for the next extraction."
        }
        tools::BREW_START => "Begins extraction.",
        tools::BREW_STOP => {
            "Ends extraction. Usually unnecessary where the start declared a              target mass, and the way to stop a shot that is not worth finishing."
        }
        tools::BREW_SET_LIVE_TARGET => {
            "Adjusts the target part way through an extraction."
        }
        tools::PUCK_EJECT => "Discards the spent puck.",
        tools::ADDITIVE_DISPENSE => {
            "Dispenses milk, water or syrup. A preference rather than a process              control: it changes what the drink is, not how the coffee extracted."
        }
        tools::MAINTENANCE_FLUSH => {
            "Flushes water through the group without coffee, to clear it and              bring it to temperature."
        }
        _ => return None,
    })
}

/// Tools a declaration must expose at each conformance level. ADI-P 3.1.
///
/// Lives here rather than in the validator because the names are
/// espresso-specific, and `adi-protocol` may not know them. A second domain
/// supplies its own list.
///
/// Levels are cumulative, so a level includes everything below it.
pub fn required_tools(level: adi_protocol::ConformanceLevel) -> &'static [&'static str] {
    use adi_protocol::ConformanceLevel::*;
    match level {
        // At least one signal, checked separately: Level 0 exposes no tools
        // with physical effect, so a required-tool list would be wrong.
        Advisory => &[],
        Parameterised => &[tools::BREW_START, tools::BREW_STOP],
        Autonomous => &[
            tools::BREW_START,
            tools::BREW_STOP,
            tools::GRIND_SET,
            tools::DOSE_BY_MASS,
        ],
    }
}

pub mod events {
    /// When flow begins. ADI-P 7.3 measures `shot_time` from here, and a mass
    /// curve can only be used to estimate it.
    pub const FIRST_DRIP: &str = "adi.espresso.event.first_drip";
    pub const FAULT: &str = "adi.espresso.event.fault";
}

pub mod vars {
    pub const DOSE_MASS: &str = "dose_mass";
    pub const YIELD_MASS: &str = "yield_mass";
    pub const RATIO: &str = "ratio";
    pub const SHOT_TIME: &str = "shot_time";
    pub const GRIND_SETTING: &str = "grind_setting";
    pub const BREW_TEMPERATURE: &str = "brew_temperature";
    pub const EXTRACTION_YIELD: &str = "extraction_yield";
    pub const FLOW_PEAK: &str = "flow_peak";
    /// Flow irregularity. The channeling indicator.
    pub const FLOW_VARIANCE: &str = "flow_variance";
}

/// Subjective descriptors this profile accepts. Closing this list is what keeps
/// the language layer's output schema closed.
pub const FEEDBACK_VOCABULARY: &[&str] = &[
    "sour", "balanced", "bitter", "weak", "strong", "harsh", "thin", "sweet",
];

// Preference variables are NOT listed here. They live in
// profiles/espresso/knowledge.yaml, which is the single source the engine
// reads. A copy in code could only drift from it, and a stale copy of which
// variables are preferences would let a taste request move the process.

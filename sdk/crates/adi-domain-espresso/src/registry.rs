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

/// Additives are preference variables, never extraction variables. The engine
/// must not attempt to correct a taste fault by changing them. See ADI-P 7.1.
pub const PREFERENCE_VARIABLES: &[&str] = &["milk_volume", "sugar_mass", "syrup_volume"];

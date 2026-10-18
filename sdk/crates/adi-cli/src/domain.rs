//! Choosing which profile a machine runs on.
//!
//! Every declaration states its `domain_profile`, and that is what decides.
//! This tool used to ignore it and load espresso regardless, which made the
//! whole SDK look like a coffee product: the engine has always taken a profile
//! as a runtime plug-in, and the one program anybody runs would only ever hand
//! it one.
//!
//! A profile has to be compiled into a build to be selectable here, and this
//! is a development driver rather than the binary that ships inside a machine.
//! A manufacturer with a profile of their own links the engine and names their
//! own profile in a few lines; see `HOW-TO-ADD-A-DOMAIN.md`.

use std::path::Path;

use adi_core::domain::Domain;

/// The profiles this build carries, for the message when one is not found.
const COMPILED_IN: &[&str] = &[adi_domain_base::PROFILE_ID, adi_domain_espresso::registry::PROFILE_ID];

pub fn for_profile(profile: &str) -> Result<Box<dyn Domain>, String> {
    match profile {
        adi_domain_base::PROFILE_ID => Ok(Box::new(adi_domain_base::Base::new())),
        adi_domain_espresso::registry::PROFILE_ID => {
            Ok(Box::new(adi_domain_espresso::Espresso::new()))
        }
        other => Err(format!(
            "no domain profile '{other}' is compiled into this build.\n\
             \x20       this build has: {}\n\
             \x20       a machine with no domain of its own can declare \
             domain_profile: \"{}\"\n\
             \x20       and supply its variables in a knowledge extension. \
             See HOW-TO-ADD-A-DOMAIN.md.",
            COMPILED_IN.join(", "),
            adi_domain_base::PROFILE_ID,
        )),
    }
}

/// The profile a declaration names, with a manufacturer's own knowledge over
/// it where they supplied some.
///
/// ADI-P 6.5: a failing extension is reported and the machine continues on the
/// knowledge it already holds. A bad extension degrades a machine to standard
/// behaviour and never stops it working, so this reports and carries on. That
/// is the behaviour a machine in the field must have, and therefore the
/// behaviour these tools should show.
pub fn load(
    profile: &str,
    knowledge_path: Option<&Path>,
    announce: impl Fn(&str),
) -> Result<Box<dyn Domain>, Box<dyn std::error::Error>> {
    let mut domain = for_profile(profile)?;

    if let Some(path) = knowledge_path {
        let text = std::fs::read_to_string(path)?;
        let extension: adi_protocol::knowledge::KnowledgeExtension = serde_yaml::from_str(&text)?;
        match domain.add_extension(extension, adi_protocol::knowledge::Layer::Vendor) {
            Ok(()) => announce(&format!("knowledge: ok  ({})", path.display())),
            Err(e) => eprintln!(
                "knowledge: REJECTED  {e}\n\n  The machine keeps working on base knowledge. ADI-P 6.5."
            ),
        }
    }

    Ok(domain)
}

/// The profile, or `None` with the reason printed.
///
/// `run` must refuse a profile it does not have, because it cannot reason
/// without one. `validate` and `capabilities` must not: a declaration naming a
/// profile this build was not compiled with is still a valid declaration, and
/// a validator that rejects a document for a property of the validator would
/// be reporting on itself. They say what they could not check and carry on.
pub fn load_or_report(
    profile: &str,
    knowledge_path: Option<&Path>,
) -> Option<Box<dyn Domain>> {
    match load(profile, knowledge_path, |line| println!("{line}")) {
        Ok(domain) => {
            println!("profile: {profile}");
            Some(domain)
        }
        Err(e) => {
            println!("profile: {profile}  (not in this build)
  {e}");
            None
        }
    }
}

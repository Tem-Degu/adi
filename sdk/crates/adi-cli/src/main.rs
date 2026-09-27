//! `adi`: validate protocol documents and run decisions offline.
//!
//! The validator is one of the three things a manufacturer needs in order to
//! integrate without waiting on us. See ROADMAP phase 12.

mod conformance;
mod genc;
mod models;

use std::path::PathBuf;

use adi_protocol::{Declaration, Skill};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "adi", version, about = "Artificial Domain Intelligence SDK tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a capability declaration.
    Validate {
        path: PathBuf,
        /// Also validate a skill against the declaration.
        #[arg(long)]
        skill: Option<PathBuf>,
    },
    /// Show what the engine can do with a machine: tools, and which variables
    /// it can actually move.
    Capabilities {
        path: PathBuf,
        #[arg(long)]
        skill: Option<PathBuf>,
    },
    /// Compile a declaration into C tables for the appliance-side SDK.
    ///
    /// A device never parses a declaration. This runs on a host at build time
    /// and emits const tables that land in flash. See sdk/c/DESIGN.md.
    GenerateC {
        path: PathBuf,
        /// Directory to write adip_generated_<name>.{h,c} into.
        #[arg(long, short, default_value = ".")]
        out: PathBuf,
        /// Basename for the generated files. Defaults to the declaration's
        /// model, lowercased.
        #[arg(long)]
        name: Option<String>,
    },
    /// Report on the model files: what is configured, present and recognised.
    ///
    /// The SDK ships no weights. This reads the folder rather than describing
    /// what should be in it.
    Models {
        #[command(subcommand)]
        what: ModelsCommand,
    },
    /// Run the conformance corpus.
    Conformance {
        #[arg(default_value = "conformance/espresso/cases")]
        dir: PathBuf,
        /// Answers for cases carrying spoken words rather than structured
        /// intent. Without one, such cases remain pending.
        #[arg(long)]
        interpreter: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// What is configured, what is present, and what to obtain.
    Check {
        #[arg(long, short, default_value = "models/models.yaml")]
        config: PathBuf,
        /// Defaults to manifest.yaml beside the config.
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// The checksum and size for a manifest entry.
    Hash { path: PathBuf },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path, skill } => validate(path, skill),
        Command::Capabilities { path, skill } => capabilities(path, skill),
        Command::GenerateC { path, out, name } => generate_c(path, out, name),
        Command::Models { what } => match what {
            ModelsCommand::Check { config, manifest } => models::check(config, manifest),
            ModelsCommand::Hash { path } => models::hash(path),
        },
        Command::Conformance { dir, interpreter } => conformance(dir, interpreter),
    }
}

fn load_declaration(path: &PathBuf) -> Result<Declaration, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&text)?)
}

fn load_skill(path: &PathBuf) -> Result<Skill, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&text)?)
}

fn validate(
    path: PathBuf,
    skill_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let declaration = load_declaration(&path)?;
    match declaration.validate() {
        Ok(()) => println!("declaration: ok  ({} {})", declaration.device.vendor, declaration.device.model),
        Err(e) => {
            eprintln!("declaration: FAILED  {e}");
            std::process::exit(1);
        }
    }

    if !declaration.has_stateful_envelope() {
        println!(
            "note: no invariants or rate limits declared. \
             D2 will refuse to run against this machine."
        );
    }

    if let Some(skill_path) = skill_path {
        let skill = load_skill(&skill_path)?;
        let mut unknown = 0;
        for response in &skill.responses {
            if declaration.tool(&response.tool).is_none() {
                eprintln!(
                    "skill: response references undeclared tool '{}'",
                    response.tool
                );
                unknown += 1;
            }
        }
        if unknown > 0 {
            std::process::exit(1);
        }
        println!("skill: ok  ({} responses)", skill.responses.len());
    }

    Ok(())
}

fn capabilities(
    path: PathBuf,
    skill_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let declaration = load_declaration(&path)?;
    let skill = skill_path.map(|p| load_skill(&p)).transpose()?;

    println!("tools the engine may invoke:");
    for tool in declaration.engine_tools() {
        let kind = if tool.id.is_vendor() { "vendor" } else { "registry" };
        println!("  {:<45} {}", tool.id.as_str(), kind);
    }

    if let Some(skill) = skill {
        println!("\nvariables the engine can move on this machine:");
        let mut seen: Vec<String> = skill
            .responses
            .iter()
            .map(|r| r.variable.0.clone())
            .collect();
        seen.sort();
        seen.dedup();
        for variable in seen {
            println!("  {variable}");
        }

        println!("\nvendor tools with no declared response (callable, not selectable):");
        for tool in declaration.tools.iter().filter(|t| t.id.is_vendor()) {
            if skill.responses_for(&tool.id).next().is_none() {
                println!("  {}", tool.id.as_str());
            }
        }
    }

    Ok(())
}

fn generate_c(
    path: PathBuf,
    out: PathBuf,
    name: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let declaration = load_declaration(&path)?;
    declaration.validate()?;

    let basename = name.unwrap_or_else(|| {
        declaration
            .device
            .model
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    });

    let generated = genc::generate(&declaration, &basename)?;

    std::fs::create_dir_all(&out)?;
    let h = out.join(format!("adip_generated_{basename}.h"));
    let c = out.join(format!("adip_generated_{basename}.c"));
    std::fs::write(&h, generated.header)?;
    std::fs::write(&c, generated.source)?;

    println!("wrote {}", h.display());
    println!("wrote {}", c.display());
    println!(
        "  {} tools, {} signals, {} invariants, {} rate limits",
        declaration.tools.len(),
        declaration.signals.len(),
        declaration.envelope.invariants.len(),
        declaration.envelope.rate_limits.len()
    );
    Ok(())
}

fn conformance(
    dir: PathBuf,
    interpreter: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let failures = conformance::run(&dir, interpreter.as_deref())?;
    if failures > 0 {
        // A conformance run that fails must fail the process, or it cannot be
        // wired into anyone's build.
        std::process::exit(1);
    }
    Ok(())
}

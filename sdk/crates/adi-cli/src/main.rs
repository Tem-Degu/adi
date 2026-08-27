//! `adi`: validate protocol documents and run decisions offline.
//!
//! The validator is one of the three things a manufacturer needs in order to
//! integrate without waiting on us. See ROADMAP phase 12.

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
    /// Run the conformance corpus.
    Conformance {
        #[arg(default_value = "conformance/espresso/cases")]
        dir: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path, skill } => validate(path, skill),
        Command::Capabilities { path, skill } => capabilities(path, skill),
        Command::Conformance { dir } => conformance(dir),
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

fn conformance(_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    todo!("load case files, run the domain, compare against expected decisions")
}

//! `adi models check`: what is configured, what is present, and what it is.
//!
//! A setup page states what should be in a folder. This reads what is, which
//! is the difference between documentation that can go out of date and a
//! report that cannot.

use std::path::{Path, PathBuf};

use adi_models::{hash_file, survey, Backend, Manifest, ModelsConfig, Seam, SeamStatus};

/// Report on every seam, and say what to do about anything missing.
pub fn check(config_path: PathBuf, manifest_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let config = ModelsConfig::load(&config_path)?;

    let manifest_path = manifest_path.unwrap_or_else(|| {
        config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("manifest.yaml")
    });

    let manifest = if manifest_path.exists() {
        Manifest::load(&manifest_path)?
    } else {
        println!("note: no manifest at {}", manifest_path.display());
        println!("      models will load but cannot be recognised by name\n");
        Manifest::default()
    };

    let statuses = survey(&config, &manifest);
    let mut missing = Vec::new();

    for status in &statuses {
        print_seam(status);
        if !status.ready() {
            missing.push(status.seam);
        }
    }

    if !missing.is_empty() {
        println!();
        for seam in &missing {
            suggest(*seam, &manifest, &config);
        }
    }

    let ready = statuses.iter().filter(|s| s.ready()).count();
    println!("\n{ready} of {} seams ready", statuses.len());

    if ready < statuses.len() {
        println!(
            "\nAn engine with an unserved seam refuses the parts it cannot perform\nand names the seam. It does not fail."
        );
    }

    Ok(())
}

fn print_seam(status: &SeamStatus) {
    let name = status.seam.as_str();

    match (&status.provenance, &status.problem, status.backend) {
        (_, _, Backend::None) => {
            println!("  {name:<13} not configured");
        }
        (_, Some(problem), _) => {
            println!("  {name:<13} MISSING");
            println!("  {:<13} {problem}", "");
        }
        (Some(provenance), None, backend) => {
            let mark = if provenance.verified { "verified" } else { "unverified" };
            println!(
                "  {name:<13} ok        {}  [{:?}, {mark}]",
                provenance.describe(),
                backend
            );
        }
        (None, None, backend) => {
            println!("  {name:<13} ok        [{backend:?}]");
        }
    }
}

/// For a seam that is not ready, say exactly what to obtain and from where.
fn suggest(seam: Seam, manifest: &Manifest, config: &ModelsConfig) {
    let candidates: Vec<_> = manifest.for_seam(seam).collect();
    if candidates.is_empty() {
        return;
    }

    println!("  to serve the {} seam, obtain one of:", seam.as_str());
    for model in candidates {
        println!("    {}", model.id);
        println!("      file     {}", model.file);
        println!("      from     {}", model.source);
        println!("      licence  {}", model.licence);
        println!("      sha256   {}", model.sha256);
    }

    if let Some(root) = &config.root {
        println!("    place it under {}", root.display());
    }
    println!();
}

/// `adi models hash <file>`: the checksum to put in a manifest entry.
pub fn hash(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let digest = hash_file(&path)?;
    let bytes = std::fs::metadata(&path)?.len();
    println!("sha256: \"{digest}\"");
    println!("bytes: {bytes}");
    Ok(())
}

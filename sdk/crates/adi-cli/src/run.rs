//! `adi run`: a live session against a real appliance.
//!
//! Everything else in this tool inspects files or replays a corpus. This
//! starts an appliance, opens a session, and lets a person drive it, which is
//! the only way to see what the SDK actually does.
//!
//! The appliance is a real build of `libadip` running as a separate process.
//! Every byte crosses the same framing, CRC, decoding, range validation,
//! interlock check and stateful envelope that would run on a machine. Nothing
//! here is simulated except the mechanism at the far end.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use adi_core::{Cycle, SubjectiveFeedback};
use adi_decision::{
    AutonomyLevel, Interpreter, ModelActionSelector, ModelToolSelector, SelectionContext,
    Understood,
};
use adi_domain_espresso::Espresso;
use adi_models::{
    Backend, ModelsConfig, ProcessChooser, ProcessInterpreter, ScriptedInterpreter, Seam,
};
use adi_protocol::declaration::Declaration;
use adi_protocol::ids::VariableId;
use adi_protocol::skill::Skill;
use adi_protocol::units::{Quantity, Unit};
use adi_session::{ChildAppliance, Session};

pub fn run(
    declaration_path: PathBuf,
    skill_path: PathBuf,
    appliance_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    level: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let declaration: Declaration = serde_yaml::from_str(&read(&declaration_path)?)?;
    let skill: Skill = serde_yaml::from_str(&read(&skill_path)?)?;
    let domain = Espresso::new();

    // Checked before anything is started. An argument that cannot be honoured
    // should be refused on its own terms, not after a failure to find a
    // binary that was never going to be used.
    let requested = parse_level(level.as_deref())?;

    let appliance_path = appliance_path
        .map(|given| resolve_executable(&given))
        .unwrap_or_else(default_appliance);

    if !appliance_path.exists() {
        return Err(format!(
            "no appliance at {}\n\nBuild it first:\n    cd sdk/c && make example",
            appliance_path.display()
        )
        .into());
    }

    println!(
        "  machine      {} {}",
        declaration.device.vendor, declaration.device.model
    );
    println!("  appliance    {}", appliance_path.display());

    let transport = ChildAppliance::start(&appliance_path.to_string_lossy(), &[])?;

    // ADI-P 3.3 makes D2 the default. It needs a chooser, so without one the
    // session opens at D0, which is deterministic and needs no model. The
    // level is reported either way rather than assumed, because a record
    // naming a level that did not run is the one thing 3.3 forbids.
    let chooser = load_chooser(config_path.as_deref(), requested)?;

    let mut session = match (requested, chooser) {
        (AutonomyLevel::D2, Some((chooser, label))) => {
            println!("  chooser      {label}");
            let selector = ModelActionSelector::new(chooser, &declaration)?;
            Session::open_with_selector(
                &declaration,
                Some(&skill),
                &domain,
                Box::new(selector),
                Box::new(transport),
            )
        }
        (AutonomyLevel::D1, Some((chooser, label))) => {
            println!("  chooser      {label}");
            Session::open_with_selector(
                &declaration,
                Some(&skill),
                &domain,
                Box::new(ModelToolSelector { chooser }),
                Box::new(transport),
            )
        }
        (AutonomyLevel::D0, _) => Session::open(
            &declaration,
            Some(&skill),
            &domain,
            AutonomyLevel::D0,
            Box::new(transport),
        )?,
        (wanted, None) => {
            // Printed and exited rather than returned, because a Box<dyn
            // Error> built from a String prints through Debug, which turns a
            // multi-line explanation into one line of escape sequences.
            eprintln!("
{} needs a chooser, and none is configured.
", wanted.as_str());
            eprintln!("Supply one with --config:");
            eprintln!("    cargo run -p adi-cli -- run --config models/models.example-process.yaml
");
            eprintln!("Or run deterministically with --level d0.");
            std::process::exit(2);
        }
    };

    println!("  autonomy     {}", session.level().as_str());

    let interpreter = load_interpreter(config_path.as_deref(), &domain)?;
    match &interpreter {
        Some((_, label)) => println!("  interpreter  {label}"),
        None => println!("  interpreter  none, so spoken requests are unavailable"),
    }

    println!();
    help();

    let stdin = std::io::stdin();
    let mut now_ms: u64 = 0;

    loop {
        print!("> ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        now_ms += 60_000;

        match line {
            "" => continue,
            "quit" | "exit" => break,
            "help" => help(),
            "status" => status(&mut session, now_ms)?,
            "cycle" => cycle(&mut session, now_ms)?,
            _ => request(&mut session, interpreter.as_ref(), line, now_ms)?,
        }
    }

    println!("\nclosed.");
    Ok(())
}

fn help() {
    println!("  cycle    feed a finished cycle and let the engine correct it");
    println!("           this needs no model and exercises the whole loop");
    println!("  status   send a heartbeat and report what the appliance says");
    println!("  quit     end the session");
    println!();
    println!("  anything else is treated as a spoken request, which needs an");
    println!("  interpreter. See `adi models check`.");
    println!();
}

/// The observation path: a cycle finished, something was wrong with it, and
/// the engine corrects the machine. No model is involved at any point.
fn cycle(session: &mut Session<'_>, now_ms: u64) -> Result<(), Box<dyn std::error::Error>> {
    let n = session.memory().len() + 1;

    // A short shot, which the profile diagnoses as under extracted.
    let mut settings = std::collections::BTreeMap::new();
    settings.insert(
        VariableId::from("grind_setting"),
        Quantity::new(24.0, Unit::Step),
    );
    settings.insert(VariableId::from("dose_mass"), Quantity::new(18.0, Unit::G));

    let observed = Cycle {
        cycle_id: format!("c{n}"),
        subject_id: None,
        operator_id: None,
        settings,
        measured: std::collections::BTreeMap::new(),
        traces: vec![adi_core::Trace {
            signal: adi_protocol::SemanticId::from("adi.espresso.signal.mass"),
            // Flow begins at 4 s and stops at 22 s, so an 18 second shot
            // against a 25 to 32 second target.
            samples: vec![
                (0.0, 0.0),
                (4.0, 1.0),
                (10.0, 9.0),
                (16.0, 21.0),
                (22.0, 32.0),
            ],
            unit: Unit::G,
            accuracy_abs: 0.2,
            filtered: false,
        }],
        events: Vec::new(),
        feedback: SubjectiveFeedback {
            descriptors: vec!["sour".into()],
            rating: None,
            note: None,
        },
    };

    println!("  observing    an 18 s shot, described as sour");

    let turn = session.observe(observed, "adi.espresso.style.espresso", None, now_ms)?;

    if turn.nominal {
        println!("  result       within target, nothing to do");
        return Ok(());
    }

    for (record, invocation) in turn.records.iter().zip(&turn.performed) {
        println!("  diagnosed    {}", record.cause);
        println!("  chose        {}", invocation.tool.as_str());
        for (name, value) in &invocation.numeric_arguments {
            println!("  sent         {name} = {} {:?}", value.value, value.unit);
        }
        println!("  accepted     by the appliance");
    }

    Ok(())
}

fn status(session: &mut Session<'_>, now_ms: u64) -> Result<(), Box<dyn std::error::Error>> {
    session.tick(now_ms)?;
    println!("  link         alive");
    println!("  cycles held  {}", session.memory().len());
    println!("  decisions    {}", session.history().len());
    Ok(())
}

/// The request path: a person asked for something in words.
fn request(
    session: &mut Session<'_>,
    interpreter: Option<&(Box<dyn Interpreter>, String)>,
    words: &str,
    now_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((interpreter, _)) = interpreter else {
        println!("  no interpreter is configured, so this cannot be understood.");
        println!("  run `adi models check` to see what is missing.");
        return Ok(());
    };

    let context = SelectionContext {
        history: session.history(),
        cycle: None,
        memory: Some(session.memory()),
    };

    match interpreter.interpret(words, &context)? {
        // ADI-P 9.4.3. Asking is an outcome, not a failure.
        Understood::Clarify { question, because } => {
            println!("  asks         {question}");
            println!("  because      {because}");
        }
        Understood::Intent(intent) => {
            println!("  understood   {:?}", intent.kind);
            if let Some(style) = &intent.style {
                println!("  style        {style}");
            }
            for (variable, target) in &intent.targets {
                println!("  target       {} {}", variable.0, describe(target));
            }

            let turn = session.request(&intent, now_ms)?;
            if turn.performed.is_empty() {
                println!("  result       nothing needed doing");
            }
            for invocation in &turn.performed {
                println!("  sent         {}", invocation.tool.as_str());
                println!("  accepted     by the appliance");
            }
        }
    }

    Ok(())
}

fn describe(target: &adi_protocol::intent::TargetSpec) -> String {
    use adi_protocol::intent::{Reference, TargetSpec};
    match target {
        TargetSpec::Absolute(q) => format!("= {} {:?}", q.value, q.unit),
        TargetSpec::Relative(r) => {
            let of = match &r.of {
                Reference::Last => "last".to_string(),
                Reference::Cycle(id) => id.clone(),
            };
            match (r.scale, &r.offset) {
                (Some(s), _) => format!("= {s} x {of}"),
                (None, Some(o)) => format!("= {of} {:+} {:?}", o.value, o.unit),
                (None, None) => format!("= same as {of}"),
            }
        }
    }
}

/// Which level to run at.
///
/// ADI-P 3.3 makes D2 the default. Stated explicitly rather than inferred from
/// what happens to be configured, so a developer who asked for a level and did
/// not get it is told rather than left to read the banner.
fn parse_level(requested: Option<&str>) -> Result<AutonomyLevel, Box<dyn std::error::Error>> {
    Ok(match requested.map(str::to_lowercase).as_deref() {
        None | Some("d2") => AutonomyLevel::D2,
        Some("d1") => AutonomyLevel::D1,
        Some("d0") => AutonomyLevel::D0,
        Some(other) => return Err(format!("unknown level '{other}'; expected d0, d1 or d2").into()),
    })
}

/// Start the chooser a model-driven level needs.
///
/// D0 needs none, so this returns nothing for it rather than starting a
/// program that would never be asked anything.
fn load_chooser(
    config_path: Option<&Path>,
    level: AutonomyLevel,
) -> Result<Option<(ProcessChooser, String)>, Box<dyn std::error::Error>> {
    if level == AutonomyLevel::D0 {
        return Ok(None);
    }

    let Some(path) = config_path else {
        return Ok(None);
    };

    let config = ModelsConfig::load(path)?;
    let seam = config.seam(Seam::Interpreter);

    // The chooser runs the same program as the interpreter. One model answers
    // both questions, which is what an appliance with one model resident
    // actually does.
    if seam.backend != Backend::Process {
        return Ok(None);
    }

    let command = seam.command.clone().ok_or("the process backend names no command")?;
    let tag = if level == AutonomyLevel::D2 { "d2" } else { "d1" };
    let label = format!("process ({command}) at {tag}");

    Ok(Some((
        ProcessChooser::start(&command, &seam.args, tag)?,
        label,
    )))
}

/// An interpreter and a description of where it came from, for the banner.
type Loaded = Option<(Box<dyn Interpreter>, String)>;

/// Build whatever interpreter the configuration names.
fn load_interpreter(
    config_path: Option<&Path>,
    domain: &Espresso,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    let Some(path) = config_path else {
        return Ok(None);
    };

    let config = ModelsConfig::load(path)?;
    let seam = config.seam(Seam::Interpreter);

    match seam.backend {
        Backend::None => Ok(None),

        Backend::Scripted => {
            let model = config
                .model_path(Seam::Interpreter)
                .ok_or("the scripted backend names no answers file")?;
            let label = format!("scripted ({})", model.display());
            Ok(Some((Box::new(ScriptedInterpreter::load(&model)?), label)))
        }

        Backend::Process => {
            let command = seam.command.clone().ok_or("the process backend names no command")?;
            let label = format!("process ({command})");

            use adi_core::Domain;
            let knowledge = domain.knowledge();
            let interpreter = ProcessInterpreter::start(
                &command,
                &seam.args,
                knowledge.style_ids().into_iter().map(str::to_string).collect(),
                knowledge.material_ids().into_iter().map(str::to_string).collect(),
                domain.variables().iter().map(|v| v.0.clone()).collect(),
                Default::default(),
            )?;
            Ok(Some((Box::new(interpreter), label)))
        }

        other => Err(format!(
            "the '{}' backend is not implemented; use 'process'",
            other.as_str()
        )
        .into()),
    }
}

fn read(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()).into())
}

fn default_appliance() -> PathBuf {
    resolve_executable(Path::new("c/build/example"))
}

/// Accept a path written without a platform's executable suffix.
///
/// A path given in documentation or a script is written once and run on
/// several platforms. `c/build/example` is the binary on Linux and macOS and
/// `example.exe` on Windows, and refusing one because it was written as the
/// other is a failure with no useful cause to report.
fn resolve_executable(given: &Path) -> PathBuf {
    if given.exists() {
        return given.to_path_buf();
    }

    let with_exe = given.with_extension(std::env::consts::EXE_EXTENSION);
    if with_exe.exists() {
        return with_exe;
    }

    given.to_path_buf()
}

//! Compile a capability declaration into C tables.
//!
//! A device never parses a declaration. This runs on a host at build time and
//! emits `const` tables that land in flash, so the runtime is an interpreter
//! over static data rather than a parser. See `sdk/c/DESIGN.md` section 3.
//!
//! It also removes a duplication that would otherwise be a drift risk: today a
//! parameter range is written in the declaration and again in hand-written C.
//! Generated from one source, the two cannot disagree.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use adi_protocol::declaration::{Authority, ConformanceLevel, Parameter};
use adi_protocol::units::{Monotonic, Unit};
use adi_protocol::Declaration;

/// Largest number of preconditions or interlocks a declaration may carry.
/// Both are addressed by bit index so a device holds each in one word.
const MAX_BITS: usize = 32;

/// Decimal places a value may use before the generator refuses it.
///
/// Rejecting rather than rounding is deliberate. A safety limit that cannot be
/// represented exactly must not be silently approximated.
const MAX_DECIMALS: i32 = 6;

#[derive(Debug)]
pub enum GenError {
    NotRepresentable { context: String, value: f64 },
    TooManyBits { kind: &'static str, count: usize },
    UnknownTool { context: String, id: String },
    UnknownParam { tool: String, param: String },
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenError::NotRepresentable { context, value } => write!(
                f,
                "{context}: {value} needs more than {MAX_DECIMALS} decimal places. \
                 Values are compiled to exact scaled integers, so this is refused \
                 rather than rounded."
            ),
            GenError::TooManyBits { kind, count } => write!(
                f,
                "{count} {kind} declared, but the device addresses them by bit \
                 index and the limit is {MAX_BITS}"
            ),
            GenError::UnknownTool { context, id } => {
                write!(f, "{context} references undeclared tool '{id}'")
            }
            GenError::UnknownParam { tool, param } => {
                write!(f, "tool '{tool}' has no parameter '{param}'")
            }
        }
    }
}

impl std::error::Error for GenError {}

/// A number as the C side stores it: mantissa * 10^exponent.
#[derive(Clone, Copy)]
struct Scaled {
    mantissa: i32,
    exponent: i8,
}

/// Convert to an exact scaled integer, or refuse.
fn scale(value: f64, context: &str) -> Result<Scaled, GenError> {
    if !value.is_finite() {
        return Err(GenError::NotRepresentable {
            context: context.to_string(),
            value,
        });
    }

    for decimals in 0..=MAX_DECIMALS {
        let factor = 10f64.powi(decimals);
        let scaled = value * factor;
        let rounded = scaled.round();

        // Exactness check. The tolerance absorbs binary representation error
        // in the input, not genuine extra precision.
        if (scaled - rounded).abs() < 1e-6 && rounded.abs() <= i32::MAX as f64 {
            return Ok(Scaled {
                mantissa: rounded as i32,
                exponent: -decimals as i8,
            });
        }
    }

    Err(GenError::NotRepresentable {
        context: context.to_string(),
        value,
    })
}

fn unit_token(unit: Unit) -> &'static str {
    match unit {
        Unit::None => "ADIP_UNIT_NONE",
        Unit::G => "ADIP_UNIT_G",
        Unit::GPerS => "ADIP_UNIT_G_PER_S",
        Unit::Ml => "ADIP_UNIT_ML",
        Unit::MlPerS => "ADIP_UNIT_ML_PER_S",
        Unit::C => "ADIP_UNIT_C",
        Unit::Bar => "ADIP_UNIT_BAR",
        Unit::S => "ADIP_UNIT_S",
        Unit::Ms => "ADIP_UNIT_MS",
        Unit::Step => "ADIP_UNIT_STEP",
        Unit::KgF => "ADIP_UNIT_KG_F",
        Unit::Pct => "ADIP_UNIT_PCT",
    }
}

fn num_literal(s: Scaled, unit: Unit) -> String {
    format!(
        "{{ {}, {}, {} }}",
        s.mantissa,
        s.exponent,
        unit_token(unit)
    )
}

fn zero_num() -> String {
    "{ 0, 0, ADIP_UNIT_NONE }".to_string()
}

/// C string literal, or `NULL` for absent.
fn cstr(value: Option<&str>) -> String {
    match value {
        None => "NULL".to_string(),
        Some(s) => {
            let mut out = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                        // The wire and the tables are ASCII. Anything else in
                        // an identifier is a declaration bug.
                        let _ = write!(out, "\\x{:02x}", c as u32 & 0xff);
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
    }
}

/// Turn a semantic id into a C identifier fragment.
fn ident(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_uppercase()
}

pub struct Generated {
    pub header: String,
    pub source: String,
}

pub fn generate(declaration: &Declaration, basename: &str) -> Result<Generated, GenError> {
    let env = &declaration.envelope;

    // ---------------------------------------------------------- bit tables
    //
    // Preconditions are declared as free text on each tool. Collect the
    // distinct strings into one table so a tool can reference them as a
    // bitmask, and the device can hold all of them in a single word.
    let mut preconditions: Vec<String> = Vec::new();
    for tool in &declaration.tools {
        for p in &tool.preconditions {
            if !preconditions.contains(p) {
                preconditions.push(p.clone());
            }
        }
    }
    if preconditions.len() > MAX_BITS {
        return Err(GenError::TooManyBits {
            kind: "preconditions",
            count: preconditions.len(),
        });
    }
    if env.interlocks.len() > MAX_BITS {
        return Err(GenError::TooManyBits {
            kind: "interlocks",
            count: env.interlocks.len(),
        });
    }

    let tool_index: BTreeMap<&str, usize> = declaration
        .tools
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();

    // ------------------------------------------------------------- header
    let mut h = String::new();
    writeln!(h, "/* Generated by `adi generate-c`. Do not edit.").unwrap();
    writeln!(h, " *").unwrap();
    writeln!(
        h,
        " * Source declaration: {} {}",
        declaration.device.vendor, declaration.device.model
    )
    .unwrap();
    writeln!(h, " * declaration_id: {}", declaration.device.declaration_id).unwrap();
    writeln!(
        h,
        " * Regenerate rather than editing, or the tables will drift from the"
    )
    .unwrap();
    writeln!(h, " * declaration the engine reads.").unwrap();
    writeln!(h, " */").unwrap();
    writeln!(h).unwrap();
    let ns = ident(basename);
    let guard = format!("ADIP_GENERATED_{ns}_H");
    writeln!(h, "#ifndef {guard}").unwrap();
    writeln!(h, "#define {guard}").unwrap();
    writeln!(h).unwrap();
    writeln!(h, "#include \"adip_tables.h\"").unwrap();
    writeln!(h).unwrap();
    writeln!(h, "#ifdef __cplusplus").unwrap();
    writeln!(h, "extern \"C\" {{").unwrap();
    writeln!(h, "#endif").unwrap();
    writeln!(h).unwrap();

    writeln!(h, "/* Sizing constants, so the caller can allocate statically. */")
        .unwrap();
    writeln!(h, "#define ADIP_{ns}_TOOL_COUNT         {}", declaration.tools.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_SIGNAL_COUNT       {}", declaration.signals.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_PRECONDITION_COUNT {}", preconditions.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_INTERLOCK_COUNT    {}", env.interlocks.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_INVARIANT_COUNT    {}", env.invariants.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_RATE_LIMIT_COUNT   {}", env.rate_limits.len()).unwrap();
    writeln!(h, "#define ADIP_{ns}_RESOURCE_COUNT     {}", env.resources.len()).unwrap();
    let max_params = declaration
        .tools
        .iter()
        .map(|t| t.parameters.len())
        .max()
        .unwrap_or(0);
    writeln!(h, "#define ADIP_{ns}_MAX_PARAMS         {max_params}").unwrap();
    writeln!(h).unwrap();

    writeln!(h, "/* Tool indices. Stable for a given declaration_id. */").unwrap();
    writeln!(h, "enum {{").unwrap();
    for (i, tool) in declaration.tools.iter().enumerate() {
        writeln!(h, "    ADIP_TOOL_{}_{} = {},", ns, ident(&tool.id.0), i).unwrap();
    }
    writeln!(h, "    ADIP_TOOL_{}__COUNT = {}", ns, declaration.tools.len()).unwrap();
    writeln!(h, "}};").unwrap();
    writeln!(h).unwrap();

    if !preconditions.is_empty() {
        writeln!(h, "/* Precondition bit indices. */").unwrap();
        writeln!(h, "enum {{").unwrap();
        for (i, p) in preconditions.iter().enumerate() {
            writeln!(h, "    ADIP_PRECOND_{}_{} = {},   /* {} */", ns, ident(p), i, p).unwrap();
        }
        writeln!(h, "    ADIP_PRECOND_{}__COUNT = {}", ns, preconditions.len()).unwrap();
        writeln!(h, "}};").unwrap();
        writeln!(h).unwrap();
    }

    if !env.interlocks.is_empty() {
        writeln!(h, "/* Interlock bit indices. */").unwrap();
        writeln!(h, "enum {{").unwrap();
        for (i, il) in env.interlocks.iter().enumerate() {
            writeln!(h, "    ADIP_INTERLOCK_{}_{} = {},", ns, ident(&il.id), i).unwrap();
        }
        writeln!(h, "    ADIP_INTERLOCK_{}__COUNT = {}", ns, env.interlocks.len()).unwrap();
        writeln!(h, "}};").unwrap();
        writeln!(h).unwrap();
    }

    writeln!(
        h,
        "extern const adip_device_t adip_device_{};",
        basename
    )
    .unwrap();
    writeln!(h).unwrap();
    writeln!(h, "#ifdef __cplusplus").unwrap();
    writeln!(h, "}}").unwrap();
    writeln!(h, "#endif").unwrap();
    writeln!(h).unwrap();
    writeln!(h, "#endif /* {guard} */").unwrap();

    // ------------------------------------------------------------- source
    let mut c = String::new();
    writeln!(c, "/* Generated by `adi generate-c`. Do not edit. */").unwrap();
    writeln!(c).unwrap();
    writeln!(c, "#include \"adip_generated_{basename}.h\"").unwrap();
    writeln!(c).unwrap();
    writeln!(c, "#include <stddef.h>").unwrap();
    writeln!(c).unwrap();

    // Enum value arrays first. They are referenced from the parameter tables
    // below, and C does not allow a declaration inside an initialiser list.
    {
        let mut any = false;
        for tool in &declaration.tools {
            for (name, param) in &tool.parameters {
                if let Parameter::Enum { values, .. } = param {
                    any = true;
                    writeln!(
                        c,
                        "static const char *const enum_{}_{}[] = {{",
                        ident(&tool.id.0),
                        ident(name)
                    )
                    .unwrap();
                    for v in values {
                        writeln!(c, "    {},", cstr(Some(v))).unwrap();
                    }
                    writeln!(c, "}};").unwrap();
                }
            }
        }
        if any {
            writeln!(c).unwrap();
        }
    }

    // Parameters, one array per tool.
    for tool in &declaration.tools {
        if tool.parameters.is_empty() {
            continue;
        }
        writeln!(
            c,
            "static const adip_param_t params_{}[] = {{",
            ident(&tool.id.0)
        )
        .unwrap();
        for (name, param) in &tool.parameters {
            let ctx = format!("{}.{}", tool.id, name);
            let (kind, unit, min, max, res, dflt, flags, mono, enum_vals) = match param {
                Parameter::Number {
                    unit,
                    min,
                    max,
                    resolution,
                    default,
                    optional,
                } => {
                    let mut f = Vec::new();
                    if *optional {
                        f.push("ADIP_PARAM_F_OPTIONAL");
                    }
                    if default.is_some() {
                        f.push("ADIP_PARAM_F_HAS_DEFAULT");
                    }
                    (
                        "ADIP_PARAM_NUMBER",
                        *unit,
                        num_literal(scale(*min, &ctx)?, *unit),
                        num_literal(scale(*max, &ctx)?, *unit),
                        match resolution {
                            Some(r) => num_literal(scale(*r, &ctx)?, *unit),
                            None => zero_num(),
                        },
                        match default {
                            Some(d) => num_literal(scale(*d, &ctx)?, *unit),
                            None => zero_num(),
                        },
                        f,
                        "ADIP_MONOTONIC_UNSET".to_string(),
                        None,
                    )
                }
                Parameter::Integer {
                    unit,
                    min,
                    max,
                    resolution,
                    monotonic,
                    optional,
                } => {
                    let mut f = Vec::new();
                    if *optional {
                        f.push("ADIP_PARAM_F_OPTIONAL");
                    }
                    let m = match monotonic {
                        Some(Monotonic::HigherIsMore) => "ADIP_MONOTONIC_HIGHER_IS_MORE",
                        Some(Monotonic::HigherIsLess) => "ADIP_MONOTONIC_HIGHER_IS_LESS",
                        Some(Monotonic::ProfileDefined) => "ADIP_MONOTONIC_PROFILE",
                        None => "ADIP_MONOTONIC_UNSET",
                    };
                    (
                        "ADIP_PARAM_INTEGER",
                        *unit,
                        num_literal(scale(*min as f64, &ctx)?, *unit),
                        num_literal(scale(*max as f64, &ctx)?, *unit),
                        num_literal(scale(resolution.unwrap_or(1) as f64, &ctx)?, *unit),
                        zero_num(),
                        f,
                        m.to_string(),
                        None,
                    )
                }
                Parameter::Enum {
                    values,
                    default,
                    optional,
                } => {
                    let mut f = Vec::new();
                    if *optional {
                        f.push("ADIP_PARAM_F_OPTIONAL");
                    }
                    if default.is_some() {
                        f.push("ADIP_PARAM_F_HAS_DEFAULT");
                    }
                    (
                        "ADIP_PARAM_ENUM",
                        Unit::None,
                        zero_num(),
                        zero_num(),
                        zero_num(),
                        zero_num(),
                        f,
                        "ADIP_MONOTONIC_UNSET".to_string(),
                        Some(values.clone()),
                    )
                }
                Parameter::Object { optional, .. } => {
                    let mut f = Vec::new();
                    if *optional {
                        f.push("ADIP_PARAM_F_OPTIONAL");
                    }
                    (
                        "ADIP_PARAM_OBJECT",
                        Unit::None,
                        zero_num(),
                        zero_num(),
                        zero_num(),
                        zero_num(),
                        f,
                        "ADIP_MONOTONIC_UNSET".to_string(),
                        None,
                    )
                }
            };

            // The array itself was emitted above the parameter tables. A
            // declaration cannot appear inside an initialiser list.
            let enum_ref = match &enum_vals {
                Some(vals) => (
                    format!("enum_{}_{}", ident(&tool.id.0), ident(name)),
                    vals.len(),
                ),
                None => ("NULL".to_string(), 0usize),
            };

            let flag_expr = if flags.is_empty() {
                "0".to_string()
            } else {
                flags.join(" | ")
            };

            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .name = {},", cstr(Some(name))).unwrap();
            writeln!(c, "        .kind = {kind},").unwrap();
            writeln!(c, "        .flags = {flag_expr},").unwrap();
            writeln!(c, "        .monotonic = {mono},").unwrap();
            writeln!(c, "        .enum_count = {},", enum_ref.1).unwrap();
            writeln!(c, "        .min = {min},").unwrap();
            writeln!(c, "        .max = {max},").unwrap();
            writeln!(c, "        .resolution = {res},").unwrap();
            writeln!(c, "        .dflt = {dflt},").unwrap();
            writeln!(c, "        .enum_values = {},", enum_ref.0).unwrap();
            writeln!(c, "    }},").unwrap();
            let _ = unit;
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Effects arrays. Effects name domain variables; the device carries them
    // as declared strings for reporting, indexed here by position.
    let mut variables: Vec<String> = Vec::new();
    for tool in &declaration.tools {
        for e in &tool.effects {
            if !variables.contains(&e.0) {
                variables.push(e.0.clone());
            }
        }
    }
    if !variables.is_empty() {
        writeln!(c, "/* Domain variables referenced by effects. */").unwrap();
        writeln!(c, "static const char *const variables[] = {{").unwrap();
        for v in &variables {
            writeln!(c, "    {},", cstr(Some(v))).unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    for tool in &declaration.tools {
        if tool.effects.is_empty() {
            continue;
        }
        writeln!(
            c,
            "static const uint16_t effects_{}[] = {{ {} }};",
            ident(&tool.id.0),
            tool.effects
                .iter()
                .map(|e| variables.iter().position(|v| v == &e.0).unwrap().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    if declaration.tools.iter().any(|t| !t.effects.is_empty()) {
        writeln!(c).unwrap();
    }

    // Tools.
    writeln!(c, "static const adip_tool_t tools[] = {{").unwrap();
    for tool in &declaration.tools {
        let mut mask: u32 = 0;
        for p in &tool.preconditions {
            let bit = preconditions.iter().position(|x| x == p).unwrap();
            mask |= 1u32 << bit;
        }

        let mut flags = Vec::new();
        if tool.reversible {
            flags.push("ADIP_TOOL_F_REVERSIBLE");
        }
        if tool.abortable {
            flags.push("ADIP_TOOL_F_ABORTABLE");
        }
        if tool.implicit {
            flags.push("ADIP_TOOL_F_IMPLICIT");
        }
        if tool.authority.contains(&Authority::Engine) {
            flags.push("ADIP_TOOL_F_AUTH_ENGINE");
        }
        if tool.authority.contains(&Authority::Operator) {
            flags.push("ADIP_TOOL_F_AUTH_OPERATOR");
        }
        let flag_expr = if flags.is_empty() {
            "0".to_string()
        } else {
            flags.join(" | ")
        };

        writeln!(c, "    {{").unwrap();
        writeln!(c, "        .id = {},", cstr(Some(&tool.id.0))).unwrap();
        writeln!(c, "        .name = {},", cstr(tool.name.as_deref())).unwrap();
        if tool.parameters.is_empty() {
            writeln!(c, "        .params = NULL,").unwrap();
        } else {
            writeln!(c, "        .params = params_{},", ident(&tool.id.0)).unwrap();
        }
        writeln!(c, "        .param_count = {},", tool.parameters.len()).unwrap();
        writeln!(c, "        .flags = {flag_expr},").unwrap();
        writeln!(c, "        .precondition_mask = 0x{mask:08x}u,").unwrap();
        if tool.effects.is_empty() {
            writeln!(c, "        .effects = NULL,").unwrap();
        } else {
            writeln!(c, "        .effects = effects_{},", ident(&tool.id.0)).unwrap();
        }
        writeln!(c, "        .effect_count = {},", tool.effects.len()).unwrap();
        writeln!(
            c,
            "        .duration_ms_typ = {},",
            tool.duration_ms_typ.unwrap_or(0)
        )
        .unwrap();
        writeln!(
            c,
            "        .duration_ms_max = {},",
            tool.duration_ms_max.unwrap_or(0)
        )
        .unwrap();
        writeln!(c, "    }},").unwrap();
    }
    writeln!(c, "}};").unwrap();
    writeln!(c).unwrap();

    // Signals.
    if !declaration.signals.is_empty() {
        writeln!(c, "static const adip_signal_t signals[] = {{").unwrap();
        for s in &declaration.signals {
            let ctx = format!("signal {}", s.id);
            let acc = scale(s.accuracy.abs, &ctx)?;
            let (rmin, rmax) = match &s.range {
                Some(r) => (
                    num_literal(scale(r.min, &ctx)?, r.unit),
                    num_literal(scale(r.max, &ctx)?, r.unit),
                ),
                None => (zero_num(), zero_num()),
            };
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .id = {},", cstr(Some(&s.id.0))).unwrap();
            writeln!(c, "        .unit = {},", unit_token(s.unit)).unwrap();
            writeln!(c, "        .filtered = {},", u8::from(s.filtered)).unwrap();
            writeln!(
                c,
                "        .sample_rate_mhz = {},",
                (s.sample_rate_hz * 1000.0).round() as u32
            )
            .unwrap();
            writeln!(
                c,
                "        .accuracy_abs = {},",
                num_literal(acc, s.accuracy.unit)
            )
            .unwrap();
            writeln!(c, "        .range_min = {rmin},").unwrap();
            writeln!(c, "        .range_max = {rmax},").unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Preconditions and interlocks.
    if !preconditions.is_empty() {
        writeln!(c, "static const adip_precondition_t preconditions[] = {{").unwrap();
        for p in &preconditions {
            writeln!(c, "    {{ .expr = {} }},", cstr(Some(p))).unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    if !env.interlocks.is_empty() {
        writeln!(c, "static const adip_interlock_t interlocks[] = {{").unwrap();
        for il in &env.interlocks {
            let mask = if il.blocks.iter().any(|b| b == "*") {
                "ADIP_BLOCKS_ALL".to_string()
            } else {
                let mut m: u32 = 0;
                for b in &il.blocks {
                    match tool_index.get(b.as_str()) {
                        Some(i) => m |= 1u32 << i,
                        None => {
                            return Err(GenError::UnknownTool {
                                context: format!("interlock '{}'", il.id),
                                id: b.clone(),
                            })
                        }
                    }
                }
                format!("0x{m:08x}u")
            };
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .id = {},", cstr(Some(&il.id))).unwrap();
            writeln!(c, "        .blocks_mask = {mask},").unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Hard limits.
    if !env.limits.is_empty() {
        writeln!(c, "static const adip_hard_limit_t limits[] = {{").unwrap();
        for l in &env.limits {
            let ctx = format!("limit on '{}'", l.parameter);
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .parameter = {},", cstr(Some(&l.parameter))).unwrap();
            writeln!(
                c,
                "        .hard_min = {},",
                match l.hard_min {
                    Some(v) => num_literal(scale(v, &ctx)?, l.unit),
                    None => zero_num(),
                }
            )
            .unwrap();
            writeln!(
                c,
                "        .hard_max = {},",
                match l.hard_max {
                    Some(v) => num_literal(scale(v, &ctx)?, l.unit),
                    None => zero_num(),
                }
            )
            .unwrap();
            writeln!(c, "        .has_min = {},", u8::from(l.hard_min.is_some())).unwrap();
            writeln!(c, "        .has_max = {},", u8::from(l.hard_max.is_some())).unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Invariants.
    for inv in &env.invariants {
        if inv.accumulates.is_empty() {
            continue;
        }
        writeln!(
            c,
            "static const adip_accum_t accum_{}[] = {{",
            ident(&inv.id)
        )
        .unwrap();
        for (tool_id, param) in &inv.accumulates {
            let ti = *tool_index
                .get(tool_id.as_str())
                .ok_or_else(|| GenError::UnknownTool {
                    context: format!("invariant '{}'", inv.id),
                    id: tool_id.clone(),
                })?;
            let pi = declaration.tools[ti]
                .parameters
                .keys()
                .position(|k| k == param)
                .ok_or_else(|| GenError::UnknownParam {
                    tool: tool_id.clone(),
                    param: param.clone(),
                })?;
            writeln!(c, "    {{ .tool = {ti}, .param = {pi} }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        if !inv.reset_on.is_empty() {
            let idxs: Result<Vec<String>, GenError> = inv
                .reset_on
                .iter()
                .map(|t| {
                    tool_index
                        .get(t.as_str())
                        .map(|i| i.to_string())
                        .ok_or_else(|| GenError::UnknownTool {
                            context: format!("invariant '{}' reset_on", inv.id),
                            id: t.clone(),
                        })
                })
                .collect();
            writeln!(
                c,
                "static const uint16_t reset_{}[] = {{ {} }};",
                ident(&inv.id),
                idxs?.join(", ")
            )
            .unwrap();
        }
        writeln!(c).unwrap();
    }

    if !env.invariants.is_empty() {
        writeln!(c, "static const adip_invariant_t invariants[] = {{").unwrap();
        for inv in &env.invariants {
            let ctx = format!("invariant '{}'", inv.id);
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .id = {},", cstr(Some(&inv.id))).unwrap();
            writeln!(c, "        .quantity = {},", cstr(Some(&inv.quantity))).unwrap();
            writeln!(
                c,
                "        .min = {},",
                match inv.min {
                    Some(v) => num_literal(scale(v, &ctx)?, inv.unit),
                    None => zero_num(),
                }
            )
            .unwrap();
            writeln!(
                c,
                "        .max = {},",
                match inv.max {
                    Some(v) => num_literal(scale(v, &ctx)?, inv.unit),
                    None => zero_num(),
                }
            )
            .unwrap();
            writeln!(c, "        .has_min = {},", u8::from(inv.min.is_some())).unwrap();
            writeln!(c, "        .has_max = {},", u8::from(inv.max.is_some())).unwrap();
            if inv.accumulates.is_empty() {
                writeln!(c, "        .accumulates = NULL,").unwrap();
            } else {
                writeln!(c, "        .accumulates = accum_{},", ident(&inv.id)).unwrap();
            }
            writeln!(c, "        .accum_count = {},", inv.accumulates.len()).unwrap();
            if inv.reset_on.is_empty() {
                writeln!(c, "        .reset_on = NULL,").unwrap();
            } else {
                writeln!(c, "        .reset_on = reset_{},", ident(&inv.id)).unwrap();
            }
            writeln!(c, "        .reset_count = {},", inv.reset_on.len()).unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Rate limits.
    for rl in &env.rate_limits {
        if rl.tools.iter().any(|t| t == "*") || rl.tools.is_empty() {
            continue;
        }
        let idxs: Result<Vec<String>, GenError> = rl
            .tools
            .iter()
            .map(|t| {
                tool_index
                    .get(t.as_str())
                    .map(|i| i.to_string())
                    .ok_or_else(|| GenError::UnknownTool {
                        context: format!("rate limit '{}'", rl.id),
                        id: t.clone(),
                    })
            })
            .collect();
        writeln!(
            c,
            "static const uint16_t rl_tools_{}[] = {{ {} }};",
            ident(&rl.id),
            idxs?.join(", ")
        )
        .unwrap();
    }
    if !env.rate_limits.is_empty() {
        writeln!(c).unwrap();
        writeln!(c, "static const adip_rate_limit_t rate_limits[] = {{").unwrap();
        for rl in &env.rate_limits {
            let all = rl.tools.iter().any(|t| t == "*");
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .id = {},", cstr(Some(&rl.id))).unwrap();
            if all || rl.tools.is_empty() {
                writeln!(c, "        .tools = NULL,").unwrap();
                writeln!(c, "        .tool_count = 0,").unwrap();
            } else {
                writeln!(c, "        .tools = rl_tools_{},", ident(&rl.id)).unwrap();
                writeln!(c, "        .tool_count = {},", rl.tools.len()).unwrap();
            }
            writeln!(c, "        .all_tools = {},", u8::from(all)).unwrap();
            writeln!(
                c,
                "        .max_invocations = {},",
                rl.max_invocations.unwrap_or(0)
            )
            .unwrap();
            writeln!(
                c,
                "        .window_ms = {},",
                (rl.window_s.unwrap_or(0.0) * 1000.0).round() as u32
            )
            .unwrap();
            writeln!(
                c,
                "        .min_interval_ms = {},",
                (rl.min_interval_s.unwrap_or(0.0) * 1000.0).round() as u32
            )
            .unwrap();
            writeln!(
                c,
                "        .max_duty_centipct = {},",
                (rl.max_duty_pct.unwrap_or(0.0) * 100.0).round() as u16
            )
            .unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // Resources.
    for r in &env.resources {
        if r.consumed_by.is_empty() {
            continue;
        }
        writeln!(c, "static const adip_accum_t consume_{}[] = {{", ident(&r.id)).unwrap();
        for (tool_id, param) in &r.consumed_by {
            let ti = *tool_index
                .get(tool_id.as_str())
                .ok_or_else(|| GenError::UnknownTool {
                    context: format!("resource '{}'", r.id),
                    id: tool_id.clone(),
                })?;
            let pi = declaration.tools[ti]
                .parameters
                .keys()
                .position(|k| k == param)
                .ok_or_else(|| GenError::UnknownParam {
                    tool: tool_id.clone(),
                    param: param.clone(),
                })?;
            writeln!(c, "    {{ .tool = {ti}, .param = {pi} }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
    }
    if !env.resources.is_empty() {
        writeln!(c).unwrap();
        writeln!(c, "static const adip_resource_t resources[] = {{").unwrap();
        for r in &env.resources {
            let ctx = format!("resource '{}'", r.id);
            writeln!(c, "    {{").unwrap();
            writeln!(c, "        .id = {},", cstr(Some(&r.id))).unwrap();
            writeln!(c, "        .unit = {},", unit_token(r.unit)).unwrap();
            writeln!(
                c,
                "        .capacity = {},",
                match r.capacity {
                    Some(v) => num_literal(scale(v, &ctx)?, r.unit),
                    None => zero_num(),
                }
            )
            .unwrap();
            writeln!(c, "        .has_capacity = {},", u8::from(r.capacity.is_some()))
                .unwrap();
            if r.consumed_by.is_empty() {
                writeln!(c, "        .consumed_by = NULL,").unwrap();
            } else {
                writeln!(c, "        .consumed_by = consume_{},", ident(&r.id)).unwrap();
            }
            writeln!(c, "        .consume_count = {},", r.consumed_by.len()).unwrap();
            writeln!(c, "    }},").unwrap();
        }
        writeln!(c, "}};").unwrap();
        writeln!(c).unwrap();
    }

    // The device itself.
    let level = match declaration.conformance_level {
        ConformanceLevel::Advisory => "ADIP_CONFORMANCE_ADVISORY",
        ConformanceLevel::Parameterised => "ADIP_CONFORMANCE_PARAMETERISED",
        ConformanceLevel::Autonomous => "ADIP_CONFORMANCE_AUTONOMOUS",
    };

    writeln!(c, "const adip_device_t adip_device_{basename} = {{").unwrap();
    writeln!(c, "    .adip_version = {},", cstr(Some(&declaration.adip_version))).unwrap();
    writeln!(c, "    .domain = {},", cstr(Some(&declaration.domain))).unwrap();
    writeln!(
        c,
        "    .domain_profile = {},",
        cstr(Some(&declaration.domain_profile))
    )
    .unwrap();
    writeln!(c, "    .conformance_level = {level},").unwrap();
    writeln!(c, "    .vendor = {},", cstr(Some(&declaration.device.vendor))).unwrap();
    writeln!(c, "    .model = {},", cstr(Some(&declaration.device.model))).unwrap();
    writeln!(
        c,
        "    .hardware_revision = {},",
        cstr(declaration.device.hardware_revision.as_deref())
    )
    .unwrap();
    writeln!(
        c,
        "    .firmware_version = {},",
        cstr(Some(&declaration.device.firmware_version))
    )
    .unwrap();
    writeln!(
        c,
        "    .declaration_id = {},",
        cstr(Some(&declaration.device.declaration_id))
    )
    .unwrap();
    writeln!(
        c,
        "    .heartbeat_interval_ms = {},",
        declaration.session.heartbeat_interval_ms
    )
    .unwrap();
    writeln!(
        c,
        "    .control_timeout_ms = {},",
        declaration.session.control_timeout_ms
    )
    .unwrap();
    writeln!(c, "    .max_concurrent = {},", declaration.session.max_concurrent).unwrap();
    writeln!(c, "    .tools = tools,").unwrap();
    writeln!(c, "    .tool_count = {},", declaration.tools.len()).unwrap();
    writeln!(
        c,
        "    .variables = {},",
        if variables.is_empty() { "NULL" } else { "variables" }
    )
    .unwrap();
    writeln!(c, "    .variable_count = {},", variables.len()).unwrap();
    writeln!(
        c,
        "    .signals = {},",
        if declaration.signals.is_empty() { "NULL" } else { "signals" }
    )
    .unwrap();
    writeln!(c, "    .signal_count = {},", declaration.signals.len()).unwrap();
    writeln!(
        c,
        "    .preconditions = {},",
        if preconditions.is_empty() { "NULL" } else { "preconditions" }
    )
    .unwrap();
    writeln!(c, "    .precondition_count = {},", preconditions.len()).unwrap();
    writeln!(
        c,
        "    .interlocks = {},",
        if env.interlocks.is_empty() { "NULL" } else { "interlocks" }
    )
    .unwrap();
    writeln!(c, "    .interlock_count = {},", env.interlocks.len()).unwrap();
    writeln!(
        c,
        "    .limits = {},",
        if env.limits.is_empty() { "NULL" } else { "limits" }
    )
    .unwrap();
    writeln!(c, "    .limit_count = {},", env.limits.len()).unwrap();
    writeln!(
        c,
        "    .invariants = {},",
        if env.invariants.is_empty() { "NULL" } else { "invariants" }
    )
    .unwrap();
    writeln!(c, "    .invariant_count = {},", env.invariants.len()).unwrap();
    writeln!(
        c,
        "    .rate_limits = {},",
        if env.rate_limits.is_empty() { "NULL" } else { "rate_limits" }
    )
    .unwrap();
    writeln!(c, "    .rate_limit_count = {},", env.rate_limits.len()).unwrap();
    writeln!(
        c,
        "    .resources = {},",
        if env.resources.is_empty() { "NULL" } else { "resources" }
    )
    .unwrap();
    writeln!(c, "    .resource_count = {},", env.resources.len()).unwrap();
    writeln!(c, "    .failsafe = {{").unwrap();
    writeln!(
        c,
        "        .max_time_to_safe_ms = {},",
        env.failsafe.max_time_to_safe_ms
    )
    .unwrap();
    writeln!(c, "        .on_control_timeout = ADIP_FAILSAFE_ABORT_TO_SAFE,").unwrap();
    writeln!(c, "        .on_signal_loss = ADIP_FAILSAFE_ABORT_TO_SAFE,").unwrap();
    writeln!(c, "        .on_fault = ADIP_FAILSAFE_ABORT_TO_SAFE,").unwrap();
    writeln!(c, "    }},").unwrap();
    writeln!(
        c,
        "    .has_stateful_envelope = {},",
        u8::from(declaration.has_stateful_envelope())
    )
    .unwrap();
    writeln!(c, "}};").unwrap();

    Ok(Generated {
        header: h,
        source: c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_exactly() {
        let s = scale(36.5, "t").unwrap();
        assert_eq!((s.mantissa, s.exponent), (365, -1));

        let s = scale(0.1, "t").unwrap();
        assert_eq!((s.mantissa, s.exponent), (1, -1));

        let s = scale(320.0, "t").unwrap();
        assert_eq!((s.mantissa, s.exponent), (320, 0));

        let s = scale(-5.25, "t").unwrap();
        assert_eq!((s.mantissa, s.exponent), (-525, -2));
    }

    /// A value needing more precision than the scaled integer can hold is
    /// refused rather than rounded. A safety limit that is approximately
    /// right is not right.
    #[test]
    fn refuses_rather_than_rounding() {
        assert!(scale(1.0 / 3.0, "t").is_err());
        assert!(scale(f64::INFINITY, "t").is_err());
        assert!(scale(f64::NAN, "t").is_err());
    }

    /// Generated identifiers must be valid C in all positions.
    #[test]
    fn identifiers_are_c_safe() {
        assert_eq!(ident("adi.espresso.brew.start"), "ADI_ESPRESSO_BREW_START");
        assert_eq!(ident("x.acme.tray-wipe"), "X_ACME_TRAY_WIPE");
        assert!(ident("state == idle")
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'));
    }

    #[test]
    fn escapes_string_literals() {
        assert_eq!(cstr(None), "NULL");
        assert_eq!(cstr(Some("plain")), "\"plain\"");
        // Raw strings throughout, so what is written is what is meant.
        assert_eq!(cstr(Some(r#"has"quote"#)), r#""has\"quote""#);
        assert_eq!(cstr(Some(r"back\slash")), r#""back\\slash""#);
        // A character outside printable ASCII is a declaration bug, and must
        // be escaped rather than emitted raw into a C literal.
        assert_eq!(cstr(Some("caf\u{e9}")), r#""caf\xe9""#);
    }
}

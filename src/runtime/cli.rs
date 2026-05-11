use crate::ir::{BesogneIR, ResolvedFlag, ResolvedFlagKind};
use crate::runtime::config;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogFormat {
    Human,
    Json,
    Ci,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    Normal,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpMode {
    Human,
    Internal,
}

#[derive(Debug)]
pub struct RuntimeArgs {
    pub log_format: LogFormat,
    pub run_mode: RunMode,
    pub dump: Option<DumpMode>,
    pub force: bool,
    pub verify: bool,
    pub subcommand: Option<String>,
    /// All resolved flag values keyed by env_var name
    pub flag_env: HashMap<String, String>,
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn global_args(besogne_name_upper: &str) -> Vec<Arg> {
    let config_env = leak_str(&format!("{besogne_name_upper}_CONFIG"));
    vec![
        Arg::new("log-format")
            .long("log-format")
            .short('l')
            .help("Output format")
            .value_parser(["human", "json", "ci"])
            .default_value("human"),
        Arg::new("config")
            .long("config")
            .short('c')
            .help("Config file path (JSON/YAML/TOML)")
            .long_help("Load flag values from a config file. Supports .json, .yaml/.yml, and .toml.\nPriority: CLI arg > env var > config file > default.\nUse nested keys for subcommand flags: { \"integration\": { \"timeout\": \"600\" } }")
            .env(config_env)
            .action(ArgAction::Set),
        Arg::new("all")
            .long("all")
            .help("Run all phases (build + pre + exec) in one shot")
            .action(ArgAction::SetTrue),
        Arg::new("force")
            .long("force")
            .short('f')
            .help("Force re-probe all preconditions (ignore cached warmup)")
            .action(ArgAction::SetTrue),
        Arg::new("verify")
            .long("verify")
            .help("Idempotency detector: run exec phase twice, compare outputs, report non-idempotent commands")
            .action(ArgAction::SetTrue),
        Arg::new("dump")
            .long("dump")
            .help("Show human-friendly summary and exit")
            .action(ArgAction::SetTrue),
        Arg::new("dump-internal")
            .long("dump-internal")
            .help("Dump raw IR as JSON and exit")
            .action(ArgAction::SetTrue),
        Arg::new("completions")
            .long("completions")
            .help("Generate shell completions and exit")
            .long_help("Generate shell completions for the given shell and print to stdout.\nSupported: bash, zsh, fish, elvish, powershell")
            .value_parser(["bash", "zsh", "fish", "elvish", "powershell"])
            .action(ArgAction::Set),
        Arg::new("man")
            .long("man")
            .help("Generate man page and exit")
            .action(ArgAction::SetTrue),
        Arg::new("markdown")
            .long("markdown")
            .help("Generate markdown documentation and exit")
            .action(ArgAction::SetTrue),
    ]
}

pub fn build_runtime_cli(ir: &BesogneIR) -> Command {
    let name = leak_str(&ir.metadata.name);
    let version = leak_str(&ir.metadata.version);
    let about = leak_str(&ir.metadata.description);
    let besogne_name_upper = ir.metadata.name.to_uppercase().replace('-', "_");

    let usage = leak_str(&format!("{name} [OPTIONS]"));
    let mut cmd = Command::new(name)
        .version(version)
        .about(about)
        .bin_name(name)
        .override_usage(usage);

    for arg in global_args(&besogne_name_upper) {
        cmd = cmd.arg(arg);
    }

    let mut subcmd_flags: HashMap<String, Vec<&ResolvedFlag>> = HashMap::new();
    let mut root_flags: Vec<&ResolvedFlag> = Vec::new();

    for flag in &ir.flags {
        if let Some(sub) = &flag.subcommand {
            subcmd_flags.entry(sub.clone()).or_default().push(flag);
        } else {
            root_flags.push(flag);
        }
    }

    for flag in &root_flags {
        cmd = cmd.arg(build_flag_arg(flag));
    }

    for (sub_name, flags) in &subcmd_flags {
        let sub_name_static = leak_str(sub_name);
        let mut sub = Command::new(sub_name_static);
        for flag in flags {
            sub = sub.arg(build_flag_arg(flag));
        }
        for arg in global_args(&besogne_name_upper) {
            sub = sub.arg(arg);
        }
        cmd = cmd.subcommand(sub);
    }

    cmd
}

fn build_flag_arg(flag: &ResolvedFlag) -> Arg {
    let name = leak_str(&flag.name);
    let env_var = leak_str(&flag.env_var);

    match &flag.kind {
        ResolvedFlagKind::Bool => {
            let mut arg = Arg::new(name)
                .long(name)
                .action(ArgAction::SetTrue)
                .env(env_var);
            if let Some(s) = flag.short {
                arg = arg.short(s);
            }
            if let Some(desc) = &flag.description {
                arg = arg.help(leak_str(desc));
            }
            if let Some(doc) = &flag.doc {
                arg = arg.long_help(leak_str(doc));
            }
            arg
        }
        ResolvedFlagKind::String => {
            let mut arg = Arg::new(name)
                .long(name)
                .action(ArgAction::Set)
                .env(env_var);
            if let Some(s) = flag.short {
                arg = arg.short(s);
            }
            if let Some(desc) = &flag.description {
                arg = arg.help(leak_str(desc));
            }
            if let Some(doc) = &flag.doc {
                arg = arg.long_help(leak_str(doc));
            }
            if flag.required {
                arg = arg.required(true);
            }
            if let Some(default) = &flag.default {
                if let Some(s) = default.as_str() {
                    arg = arg.default_value(leak_str(s));
                }
            }
            if let Some(vals) = &flag.values {
                let strs: Vec<&'static str> = vals.iter().map(|s| leak_str(s)).collect();
                arg = arg.value_parser(strs);
            }
            arg
        }
        ResolvedFlagKind::Positional => {
            let mut arg = Arg::new(name)
                .action(ArgAction::Set)
                .env(env_var);
            if let Some(desc) = &flag.description {
                arg = arg.help(leak_str(desc));
            }
            if let Some(doc) = &flag.doc {
                arg = arg.long_help(leak_str(doc));
            }
            if flag.required {
                arg = arg.required(true);
            }
            if let Some(default) = &flag.default {
                if let Some(s) = default.as_str() {
                    arg = arg.default_value(leak_str(s));
                }
            }
            if let Some(vals) = &flag.values {
                let strs: Vec<&'static str> = vals.iter().map(|s| leak_str(s)).collect();
                arg = arg.value_parser(strs);
            }
            arg
        }
    }
}

fn extract_flag_env(flags: &[&ResolvedFlag], matches: &ArgMatches) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for flag in flags {
        let name = leak_str(&flag.name);
        match &flag.kind {
            ResolvedFlagKind::Bool => {
                let val = matches.get_flag(name);
                env.insert(flag.env_var.clone(), if val { "1".into() } else { "0".into() });
            }
            ResolvedFlagKind::String | ResolvedFlagKind::Positional => {
                if let Some(val) = matches.get_one::<String>(name) {
                    env.insert(flag.env_var.clone(), val.clone());
                } else if let Some(default) = &flag.default {
                    if let Some(s) = default.as_str() {
                        env.insert(flag.env_var.clone(), s.to_string());
                    }
                }
            }
        }
    }
    env
}

/// Merge config file values into flag_env (lower priority than CLI/env).
/// Config keys match flag names. Nested keys: { "subcmd": { "flag": "val" } }
fn merge_config(
    flag_env: &mut HashMap<String, String>,
    config_values: &HashMap<String, String>,
    ir: &BesogneIR,
) {
    for flag in &ir.flags {
        // Already set by CLI/env — skip
        if flag_env.contains_key(&flag.env_var) {
            // Check if value is actually from CLI/env (non-default)
            // Config is lowest priority, so only fill in if env_var not yet set
            continue;
        }

        // Try flat key: "flag-name"
        if let Some(val) = config_values.get(&flag.name) {
            flag_env.insert(flag.env_var.clone(), val.clone());
            continue;
        }

        // Try nested key: "subcommand.flag-name"
        if let Some(sub) = &flag.subcommand {
            let nested_key = format!("{}.{}", sub, flag.name);
            if let Some(val) = config_values.get(&nested_key) {
                flag_env.insert(flag.env_var.clone(), val.clone());
            }
        }
    }
}

pub fn parse_runtime_args(ir: &BesogneIR) -> RuntimeArgs {
    let cmd = build_runtime_cli(ir);
    let matches = cmd.get_matches();

    let (active_matches, subcommand_name) = if let Some((sub_name, sub_matches)) = matches.subcommand() {
        (sub_matches, Some(sub_name.to_string()))
    } else {
        (&matches, None)
    };

    // Handle doc generation commands (exit early)
    handle_doc_generation(ir, active_matches);

    let log_format = match active_matches.get_one::<String>("log-format").map(|s| s.as_str()) {
        Some("json") => LogFormat::Json,
        Some("ci") => LogFormat::Ci,
        _ => LogFormat::Human,
    };

    let run_mode = if active_matches.get_flag("all") {
        RunMode::All
    } else {
        RunMode::Normal
    };

    let dump = if active_matches.get_flag("dump-internal") {
        Some(DumpMode::Internal)
    } else if active_matches.get_flag("dump") {
        Some(DumpMode::Human)
    } else {
        None
    };

    // Collect CLI/env flag values
    let root_flags: Vec<&ResolvedFlag> = ir.flags.iter().filter(|f| f.subcommand.is_none()).collect();
    let mut flag_env = extract_flag_env(&root_flags, &matches);

    if let Some(ref sub_name) = subcommand_name {
        let sub_flags: Vec<&ResolvedFlag> = ir
            .flags
            .iter()
            .filter(|f| f.subcommand.as_deref() == Some(sub_name))
            .collect();
        flag_env.extend(extract_flag_env(&sub_flags, active_matches));
    }

    // Load and merge config file (lowest priority)
    if let Some(config_path) = active_matches.get_one::<String>("config") {
        match config::load_config(config_path) {
            Ok(config_values) => {
                merge_config(&mut flag_env, &config_values, ir);
            }
            Err(e) => {
                eprintln!("warning: {e}");
            }
        }
    }

    let force = active_matches.get_flag("force");
    let verify = active_matches.get_flag("verify");

    RuntimeArgs {
        log_format,
        run_mode,
        dump,
        force,
        verify,
        subcommand: subcommand_name,
        flag_env,
    }
}

fn handle_doc_generation(ir: &BesogneIR, matches: &ArgMatches) {
    if let Some(shell) = matches.get_one::<String>("completions") {
        let mut cmd = build_runtime_cli(ir);
        let shell = match shell.as_str() {
            "bash" => clap_complete::Shell::Bash,
            "zsh" => clap_complete::Shell::Zsh,
            "fish" => clap_complete::Shell::Fish,
            "elvish" => clap_complete::Shell::Elvish,
            "powershell" => clap_complete::Shell::PowerShell,
            _ => {
                eprintln!("unsupported shell: {shell}");
                std::process::exit(1);
            }
        };
        clap_complete::generate(shell, &mut cmd, &ir.metadata.name, &mut std::io::stdout());
        std::process::exit(0);
    }

    if matches.get_flag("man") {
        let cmd = build_runtime_cli(ir);
        let man = clap_mangen::Man::new(cmd);
        man.render(&mut std::io::stdout()).unwrap_or_else(|e| {
            eprintln!("error generating man page: {e}");
            std::process::exit(1);
        });
        std::process::exit(0);
    }

    if matches.get_flag("markdown") {
        print_markdown_doc(ir);
        std::process::exit(0);
    }
}

fn print_markdown_doc(ir: &BesogneIR) {
    println!("# {}", ir.metadata.name);
    println!();
    println!("{}", ir.metadata.description);
    println!();
    println!("**Version:** {}", ir.metadata.version);
    println!();

    // Global flags
    println!("## Global Options");
    println!();
    println!("| Flag | Short | Env | Description |");
    println!("|------|-------|-----|-------------|");
    println!("| `--log-format` | `-l` | | Output format (human/json/ci) |");
    println!("| `--config` | `-c` | `{}_CONFIG` | Config file path (JSON/YAML/TOML) |", ir.metadata.name.to_uppercase().replace('-', "_"));
    println!("| `--all` | | | Run all phases in one shot |");
    println!("| `--dump` | | | Show human-friendly summary |");
    println!("| `--dump-internal` | | | Dump raw IR as JSON |");
    println!("| `--completions` | | | Generate shell completions |");
    println!("| `--man` | | | Generate man page |");
    println!("| `--markdown` | | | Generate this documentation |");
    println!();

    let root_flags: Vec<_> = ir.flags.iter().filter(|f| f.subcommand.is_none()).collect();
    if !root_flags.is_empty() {
        println!("## Flags");
        println!();
        print_flag_table(&root_flags);
    }

    let mut subcmds: HashMap<&str, Vec<&ResolvedFlag>> = HashMap::new();
    for flag in ir.flags.iter().filter(|f| f.subcommand.is_some()) {
        subcmds.entry(flag.subcommand.as_deref().unwrap()).or_default().push(flag);
    }
    for (sub, flags) in &subcmds {
        println!("## Subcommand: `{sub}`");
        println!();
        print_flag_table(flags);
    }

    // Inputs by phase
    let phases = [
        ("Build (sealed)", Phase::Build),
        ("Pre (preconditions)", Phase::Pre),
        ("Exec (commands)", Phase::Exec),
    ];
    use crate::manifest::Phase;
    for (label, phase) in &phases {
        let inputs: Vec<_> = ir.inputs.iter().filter(|i| &i.phase == phase).collect();
        if !inputs.is_empty() {
            println!("## {label}");
            println!();
            for i in &inputs {
                println!("- `{}`", i.id);
            }
            println!();
        }
    }
}

fn print_flag_table(flags: &[&ResolvedFlag]) {
    println!("| Flag | Short | Env | Default | Description |");
    println!("|------|-------|-----|---------|-------------|");
    for flag in flags {
        let short = flag.short.map(|s| format!("`-{s}`")).unwrap_or_default();
        let default = flag.default.as_ref()
            .map(|d| {
                if let Some(s) = d.as_str() { s.to_string() }
                else if let Some(b) = d.as_bool() { b.to_string() }
                else { d.to_string() }
            })
            .unwrap_or_default();
        let desc = flag.description.as_deref().unwrap_or("");
        let req = if flag.required { " **required**" } else { "" };
        println!(
            "| `--{}` | {short} | `{}` | {default} | {desc}{req} |",
            flag.name, flag.env_var
        );
    }
    println!();

    // Print long docs if any
    for flag in flags {
        if let Some(doc) = &flag.doc {
            println!("### `--{}`", flag.name);
            println!();
            println!("{doc}");
            println!();
        }
    }
}

//! The spectype command line, shared by the binary and the npm package's native addon.

#![expect(clippy::print_stdout, clippy::print_stderr, reason = "the CLI reports to the terminal")]

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::Instant;

use spectype_core::{Error, Format, Host, Options, generate_retained, parse_document};

mod config;
mod remote;

pub use remote::Fetch;
pub use spectype_core::{HostError, LoadedDocument};

const HELP: &str = "Usage
  $ PROGRAM [input] [options]

Options
  --help                     Display this
  --version                  Display the version
  --redocly [path], -c       Specify path to Redocly config (default: redocly.yaml)
  --output, -o               Specify output file (if not specified in redocly.yaml)
  --enum                     Export true TS enums instead of unions
  --enum-values              Export enum values as arrays
  --conditional-enums        Only generate true TS enums when enum metadata is available (default: false)
  --dedupe-enums             Dedupe enum types when `--enum=true` is set
  --check                    Check that the generated types are up-to-date. (default: false)
  --export-type, -t          Export top-level `type` instead of `interface`
  --immutable                Generate readonly types
  --additional-properties    Treat schema objects as if `additionalProperties: true` is set
  --empty-objects-unknown    Generate `unknown` instead of `Record<string, never>` for empty objects
  --default-non-nullable     Set to `false` to ignore default values when generating non-nullable types
  --properties-required-by-default
                             Treat schema objects as if `required` is set to all properties by default
  --array-length             Generate tuples using array minItems / maxItems
  --path-params-as-types     Convert paths to template literal types
  --alphabetize              Sort object keys alphabetically
  --exclude-deprecated       Exclude deprecated types
  --root-types (optional)    Export schemas types at root level
  --root-types-no-schema-prefix (optional)
                             Do not add \"Schema\" prefix to types at the root level (should only be used with --root-types)
  --root-types-keep-casing   Keep casing of root types (should only be used with --root-types)
  --make-paths-enum          Generate ApiPaths enum for all paths
  --read-write-markers       Generate $Read/$Write markers for readOnly/writeOnly properties
  --generate-path-params     Generate path parameters from the URL when they are not declared
  --fix <names>              Opt-in fixes for openapi-typescript bugs (comma-separated, or \"all\"):
                             array-length, enum-values-type, root-types-names, enum-names,
                             operation-id-refs, default-non-nullable-scope, const-items,
                             allof-empty-objects, discriminator-oneof-base, allof-required,
                             nested-discriminators, discriminator-declared-values,
                             anyof-properties
  --inject <file>            Insert the TypeScript in <file> before the generated types
";

struct Args {
    input: Option<String>,
    output: Option<String>,
    redocly: Option<String>,
    check: bool,
    help: bool,
    version: bool,
    options: Options,
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Mirrors yargs-parser: kebab-case or camelCase flags, `--flag=false`, `--no-flag`.
fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut args = Args {
        input: None,
        output: None,
        redocly: None,
        check: false,
        help: false,
        version: false,
        options: Options::default(),
    };
    let mut iter = raw.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let Some(flag) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-').filter(|_| arg.len() > 1)) else {
            if args.input.is_none() {
                args.input = Some(arg);
            }
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
            None => (flag.to_owned(), None),
        };
        let kebab: String = name
            .chars()
            .flat_map(|c| if c.is_ascii_uppercase() { vec!['-', c.to_ascii_lowercase()] } else { vec![c] })
            .collect();
        let (kebab, negated) = match kebab.strip_prefix("no-") {
            Some(rest) => (rest.to_owned(), true),
            None => (kebab, false),
        };
        if matches!(kebab.as_str(), "output" | "o" | "redocly" | "c" | "fix" | "inject") {
            let value = match inline {
                Some(v) => v,
                None => iter.next().ok_or_else(|| format!("--{kebab} needs a value"))?,
            };
            if kebab == "fix" {
                for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    args.options.fix.enable(name).map_err(|e| e.to_string())?;
                }
            } else if kebab == "inject" {
                let text = std::fs::read_to_string(&value).map_err(|e| format!("Could not read {value}: {e}"))?;
                args.options.inject = Some(text);
            } else if matches!(kebab.as_str(), "output" | "o") {
                args.output = Some(value);
            } else {
                args.redocly = Some(value);
            }
            continue;
        }
        let mut value = match &inline {
            Some(v) => parse_bool(v).ok_or_else(|| format!("invalid value for --{kebab}: {v}"))?,
            None => match iter.peek().and_then(|v| parse_bool(v).filter(|_| !v.is_empty())) {
                Some(b) => {
                    iter.next();
                    b
                }
                None => true,
            },
        };
        if negated {
            value = !value;
        }
        let o = &mut args.options;
        let slot = match kebab.as_str() {
            "help" => &mut args.help,
            "version" => &mut args.version,
            "check" => &mut args.check,
            "additional-properties" => &mut o.additional_properties,
            "alphabetize" => &mut o.alphabetize,
            "array-length" => &mut o.array_length,
            "default-non-nullable" => &mut o.default_non_nullable,
            "properties-required-by-default" => &mut o.properties_required_by_default,
            "empty-objects-unknown" => &mut o.empty_objects_unknown,
            "enum" => &mut o.enum_,
            "enum-values" => &mut o.enum_values,
            "conditional-enums" => &mut o.conditional_enums,
            "dedupe-enums" => &mut o.dedupe_enums,
            "exclude-deprecated" => &mut o.exclude_deprecated,
            "export-type" | "t" => &mut o.export_type,
            "immutable" => &mut o.immutable,
            "path-params-as-types" => &mut o.path_params_as_types,
            "root-types" => &mut o.root_types,
            "root-types-no-schema-prefix" => &mut o.root_types_no_schema_prefix,
            "root-types-keep-casing" => &mut o.root_types_keep_casing,
            "make-paths-enum" => &mut o.make_paths_enum,
            "generate-path-params" => &mut o.generate_path_params,
            "read-write-markers" => &mut o.read_write_markers,
            "immutable-types" => return Err("The --immutable-types flag has been renamed to \"--immutable\".".into()),
            "support-array-length" => {
                return Err("The --support-array-length flag has been renamed to \"--array-length\".".into());
            }
            other => {
                eprintln!("warning: unknown flag --{other} was ignored");
                continue;
            }
        };
        *slot = value;
    }
    Ok(args)
}

fn run(program: &str, raw: Vec<String>, mut host: remote::CliHost) -> Result<(), String> {
    let start = Instant::now();
    let mut args = parse_args(raw)?;
    if args.help {
        println!("{}", HELP.replacen("PROGRAM", program, 1));
        return Ok(());
    }
    if args.version {
        println!("v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.options.root_types_no_schema_prefix && !args.options.root_types {
        eprintln!("--root-types-no-schema-prefix has no effect without --root-types flag");
    }
    if args.options.root_types_keep_casing && !args.options.root_types {
        eprintln!("--root-types-keep-casing has no effect without --root-types flag");
    }

    let redocly = config::load(args.redocly.as_deref())?;
    let unsupported = redocly.lint.unsupported_rules();
    if !unsupported.is_empty() {
        eprintln!("warning: Redocly rules not supported by spectype were skipped: {}", unsupported.join(", "));
    }
    if redocly.plugins {
        eprintln!("warning: Redocly plugins are not supported by spectype and were ignored");
    }
    args.options.lint = redocly.lint.clone();
    host.headers.clone_from(&redocly.headers);
    if args.output.is_some() {
        eprintln!("✨ spectype {}", env!("CARGO_PKG_VERSION"));
    }
    if !redocly.apis.is_empty() {
        return run_apis(&redocly, &args, &mut host, start);
    }

    let quiet = args.output.is_none();
    let (output, label) = load_and_generate(args.input.as_deref(), &args.options, &mut host, quiet, true)?;
    let Some(out_path) = args.output else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(output.code.as_bytes()).and_then(|()| stdout.flush()).map_err(|e| e.to_string())?;
        return Ok(());
    };
    for w in &output.warnings {
        eprintln!(" ⚠  {w}");
    }
    if write_output(&output.code, &out_path, args.check)? {
        eprintln!("🚀 {label} → {out_path} [{:.1}ms]", start.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}

/// `redocly.yaml` `apis` mode: every API's `root` into its `x-openapi-ts.output`, both relative
/// to the config file.
fn run_apis(redocly: &config::Redocly, args: &Args, host: &mut remote::CliHost, start: Instant) -> Result<(), String> {
    if args.input.is_some() {
        eprintln!(" ⚠  APIs are specified both in Redocly Config and CLI argument. Only using Redocly config.");
    }
    if let Some(api) = redocly.apis.iter().find(|a| a.output.is_none()) {
        return Err(format!(
            "API {} is missing an `x-openapi-ts.output` key. See https://openapi-ts.dev/cli/#multiple-schemas.",
            api.name
        ));
    }
    let file = redocly.file.as_deref();
    for (i, api) in redocly.apis.iter().enumerate() {
        let shown = api.output.as_deref().unwrap_or_default();
        let root = config::relative_to_config(file, &api.root);
        let last = i + 1 == redocly.apis.len();
        let (output, _) = load_and_generate(Some(&root), &args.options, host, false, last)?;
        for w in &output.warnings {
            eprintln!(" ⚠  {w}");
        }
        if write_output(&output.code, &config::relative_to_config(file, shown), args.check)? {
            eprintln!("🚀 {} → {shown} [{:.1}ms]", api.name, start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(())
}

/// Reads `input` (a path, URL, or stdin when `None`/`-`) and generates its types. Lint problems
/// are printed here, warnings only when not `quiet`; the returned error is then empty. The
/// `last` document of a process that is exiting is left to the OS instead of being freed.
fn load_and_generate(
    input: Option<&str>,
    options: &Options,
    host: &mut remote::CliHost,
    quiet: bool,
    last: bool,
) -> Result<(spectype_core::Output, String), String> {
    let location = input.filter(|p| *p != "-").map(input_location).transpose()?;
    let (source, format, label) = if let Some(location) = &location {
        let text = if remote::is_url(location) {
            host.load(location).map_err(|e| e.to_string())?.ok_or_else(|| format!("Could not load {location}"))?.text
        } else {
            std::fs::read_to_string(location).map_err(|e| format!("Could not read {location}: {e}"))?
        };
        (text, Format::Yaml, input.unwrap_or_default().to_owned())
    } else {
        if std::io::stdin().is_terminal() {
            return Err("No input given. Pass a file path or pipe a spec into stdin (see --help)".into());
        }
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map_err(|e| format!("Could not read stdin: {e}"))?;
        (text, Format::Auto, "stdin".to_owned())
    };

    let document = parse_document(&source, format).map_err(|e| e.to_string())?;
    let skip_free = host.exiting && last;
    let (output, trees) = generate_retained(document, location.as_deref(), options, host).map_err(|e| match &e {
        Error::UnresolvedRefs { earlier, .. } => {
            for pointer in earlier {
                eprintln!(" ✘  Can't resolve $ref at {pointer}");
            }
            e.to_string()
        }
        Error::Lint(problems) => {
            for p in problems {
                if p.error {
                    eprintln!(" ✘  {p}");
                } else if !quiet {
                    eprintln!(" ⚠  {p}");
                }
            }
            String::new()
        }
        _ => e.to_string(),
    })?;
    if skip_free {
        std::mem::forget(trees);
    }
    Ok((output, label))
}

/// Writes `code` to `out_path`, or with `check` compares it; `false` when only checked.
fn write_output(code: &str, out_path: &str, check: bool) -> Result<bool, String> {
    let out = PathBuf::from(out_path);
    if check {
        let previous = std::fs::read_to_string(&out).unwrap_or_default();
        if previous != code {
            return Err("Generated types are not up-to-date!".into());
        }
        return Ok(false);
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&out, code).map_err(|e| format!("Could not write {out_path}: {e}"))?;
    Ok(true)
}

/// The input as a file path or an `http(s)` URL; `file://` URLs become paths, as upstream does.
fn input_location(input: &str) -> Result<String, String> {
    if !remote::is_url(input) && !input.starts_with("file://") {
        return Ok(input.to_owned());
    }
    let url = url::Url::parse(input).map_err(|e| format!("Invalid URL {input}: {e}"))?;
    if url.scheme() != "file" {
        return Ok(url.into());
    }
    let path = url.to_file_path().map_err(|()| format!("Invalid file URL {input}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Runs the CLI as `program` with `args` (without the program name), printing to stdout and
/// stderr, on a thread with a big stack. Returns whether it succeeded.
#[cfg(not(target_family = "wasm"))]
#[must_use]
pub fn run_cli(program: &str, args: Vec<String>) -> bool {
    spawn_cli(program, args, false)
}

/// [`run_cli`] for a process that exits as soon as it returns: the last generated document is
/// left for the OS to reclaim instead of being freed.
#[cfg(not(target_family = "wasm"))]
#[must_use]
pub fn run_cli_then_exit(program: &str, args: Vec<String>) -> bool {
    spawn_cli(program, args, true)
}

#[cfg(not(target_family = "wasm"))]
fn spawn_cli(program: &str, args: Vec<String>, exiting: bool) -> bool {
    let program = program.to_owned();
    // Deeply nested schemas recurse deeply; a big stack keeps the parser's depth limit, not the
    // OS, the thing that stops them.
    let worker = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || report(run(&program, args, remote::CliHost::new(remote::ureq_fetch(), exiting))));
    let result = match worker {
        Ok(handle) => handle.join().map_err(|_| "internal error".to_owned()),
        Err(e) => Err(format!("could not start worker thread: {e}")),
    };
    result.unwrap_or_else(|e| {
        eprintln!(" ✘  {e}");
        false
    })
}

/// [`run_cli`] on the current thread, fetching `http(s)` documents with `fetch`.
#[must_use]
pub fn run_cli_with(program: &str, args: Vec<String>, fetch: Fetch<'_>) -> bool {
    report(run(program, args, remote::CliHost::new(fetch, false)))
}

fn report(result: Result<(), String>) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            if !e.is_empty() {
                eprintln!(" ✘  {e}");
            }
            false
        }
    }
}

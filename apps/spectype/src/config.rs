//! `redocly.yaml`, read the way openapi-typescript's CLI reads it with Redocly 1.34
//! (`findConfig`, `loadConfig`): its `apis`, and the lint rules its `extends` and `rules` add up to.

use std::path::{Path, PathBuf};

use spectype_core::http::{HttpHeader, headers_of};
use spectype_core::lint::Config as LintConfig;
use spectype_core::value::{Map, Value};
use spectype_core::{Format, parse_document};

const FILE_NAMES: [&str; 4] = ["redocly.yaml", "redocly.yml", ".redocly.yaml", ".redocly.yml"];
const PRESETS: &str = include_str!("presets.json");
const FLAT_KEYS: [&str; 26] = [
    "plugins",
    "extends",
    "rules",
    "oas2Rules",
    "oas3_0Rules",
    "oas3_1Rules",
    "async2Rules",
    "async3Rules",
    "arazzo1Rules",
    "overlay1Rules",
    "preprocessors",
    "oas2Preprocessors",
    "oas3_0Preprocessors",
    "oas3_1Preprocessors",
    "async2Preprocessors",
    "async3Preprocessors",
    "arazzo1Preprocessors",
    "overlay1Preprocessors",
    "decorators",
    "oas2Decorators",
    "oas3_0Decorators",
    "oas3_1Decorators",
    "async2Decorators",
    "async3Decorators",
    "arazzo1Decorators",
    "overlay1Decorators",
];

pub struct Api {
    pub name: String,
    pub root: String,
    pub output: Option<String>,
}

#[derive(Default)]
pub struct Redocly {
    pub lint: LintConfig,
    pub apis: Vec<Api>,
    /// The config file, for resolving `root` and `x-openapi-ts.output` paths.
    pub file: Option<PathBuf>,
    pub plugins: bool,
    /// `resolve.http.headers`.
    pub headers: Vec<HttpHeader>,
}

/// `findRedocConfigPath` + `loadConfig`. Without a config file openapi-typescript calls
/// `createConfig({}, { extends: ['minimal'] })`, which drops the `extends` option (Redocly reads
/// `customExtends`), so no rule runs.
pub fn load(flag: Option<&str>) -> Result<Redocly, String> {
    let path = match flag {
        None => find(Path::new(""))?,
        Some(f) => {
            let abs = std::path::absolute(f).unwrap_or_else(|_| PathBuf::from(f));
            let found = if !abs.exists() {
                None
            } else if abs.is_dir() {
                find(&abs)?
            } else {
                Some(abs)
            };
            Some(found.ok_or_else(|| format!("Redocly config not found at: {f}"))?)
        }
    };
    let Some(path) = path else { return Ok(Redocly::default()) };
    let shown = path.display().to_string();
    let raw = read_config(&path).map_err(|e| format!("Error parsing config file at '{shown}': {e}"))?;
    let styleguide = styleguide_of(&raw)?;
    let mut stack = Vec::new();
    let merged = resolve(styleguide.as_ref(), &path, &mut stack)?;
    let plugins = merged.plugins;
    let mut apis = Vec::new();
    if let Some(Value::Object(list)) = raw.get("apis").or_else(|| raw.get("apiDefinitions")) {
        for (name, api) in list.iter() {
            let root = api.get("root").map_or_else(|| "undefined".to_owned(), Value::js_string);
            let output = api.get("x-openapi-ts").and_then(|x| x.get("output")).filter(|o| o.truthy());
            apis.push(Api { name: name.clone(), root, output: output.map(Value::js_string) });
        }
    }
    let headers = headers_of(raw.get("resolve"));
    let mut lint = merged.finish();
    lint.ignore = lint_ignore(&path);
    Ok(Redocly { lint, apis, file: Some(path), plugins, headers })
}

/// `.redocly.lint-ignore.yaml` next to the config: file → rule → pointers, files relative to it.
fn lint_ignore(config: &Path) -> std::collections::BTreeSet<(String, String, String)> {
    let mut out = std::collections::BTreeSet::new();
    let dir = config.parent().unwrap_or_else(|| Path::new(""));
    let Ok(files) = read_config(&dir.join(".redocly.lint-ignore.yaml")) else { return out };
    for (file, rules) in files.iter() {
        let path = dir.join(file);
        let path = std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().into_owned();
        for (rule, pointers) in rules.as_object().into_iter().flat_map(Map::iter) {
            for pointer in pointers.as_array().into_iter().flatten().filter_map(Value::as_str) {
                out.insert((path.clone(), rule.clone(), pointer.to_owned()));
            }
        }
    }
    out
}

/// `findConfig(dir)`.
fn find(dir: &Path) -> Result<Option<PathBuf>, String> {
    let found: Vec<PathBuf> = FILE_NAMES.iter().map(|n| dir.join(n)).filter(|p| p.exists()).collect();
    if found.len() > 1 {
        let list = found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
        return Err(format!(
            "Multiple configuration files are not allowed. Found the following files: {list}. Please use 'redocly.yaml' instead."
        ));
    }
    Ok(found.into_iter().next())
}

fn read_config(path: &Path) -> Result<Map, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    match parse_document(&text, Format::Yaml).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Ok(Map::new()),
    }
}

/// `transformConfig`'s styleguide: `styleguide`, else the flat keys, else `lint`.
fn styleguide_of(raw: &Map) -> Result<Option<Map>, String> {
    let mut flat = Map::new();
    for k in FLAT_KEYS {
        if let Some(v) = raw.get(k) {
            flat.insert(k.to_owned(), v.clone());
        }
    }
    let (lint, styleguide) = (raw.get("lint"), raw.get("styleguide"));
    if (lint.is_some() && styleguide.is_some()) || (!flat.is_empty() && (lint.is_some() || styleguide.is_some())) {
        return Err("Do not use 'lint', 'styleguide' and flat syntax together. \nSee more about the configuration in the docs: https://redocly.com/docs/cli/configuration/ \n".into());
    }
    Ok(match styleguide.or(lint) {
        Some(Value::Object(m)) => Some(m.clone()),
        Some(_) => None,
        None => (!flat.is_empty()).then_some(flat),
    })
}

#[derive(Default)]
struct Merged {
    rules: Map,
    oas3_0: Map,
    oas3_1: Map,
    plugins: bool,
}

impl Merged {
    /// `mergeExtends` for one config in the list.
    fn add(&mut self, conf: &Map) {
        let group = |k: &str| conf.get(k).and_then(Value::as_object);
        assign(&mut self.rules, group("rules"), false);
        assign(&mut self.oas3_0, group("oas3_0Rules"), false);
        assign(&mut self.oas3_0, group("rules"), true);
        assign(&mut self.oas3_1, group("oas3_1Rules"), false);
        assign(&mut self.oas3_1, group("rules"), true);
        self.plugins |= conf.get("plugins").is_some_and(|p| p.as_array().is_some_and(|a| !a.is_empty()));
    }

    /// `StyleguideConfig`: `spec` renamed to `struct`, then `{ ...rules, ...oas3_xRules }`.
    fn finish(mut self) -> LintConfig {
        for group in [&mut self.rules, &mut self.oas3_0, &mut self.oas3_1] {
            if let Some(spec) = group.get("spec").cloned() {
                group.insert("struct".into(), spec);
                *group = group.iter().filter(|(k, _)| *k != "spec").map(|(k, v)| (k.clone(), v.clone())).collect();
            }
        }
        let combine = |specific: &Map| {
            let mut out = self.rules.clone();
            for (k, v) in specific.iter() {
                out.insert(k.clone(), v.clone());
            }
            out
        };
        LintConfig { oas3_0: combine(&self.oas3_0), oas3_1: combine(&self.oas3_1), ..LintConfig::default() }
    }
}

/// `assignConfig` / `assignOnlyExistingConfig`.
fn assign(target: &mut Map, from: Option<&Map>, only_existing: bool) {
    let Some(from) = from else { return };
    for (k, v) in from.iter() {
        if only_existing && !target.contains_key(k) {
            continue;
        }
        match (target.get_mut(k), v) {
            (Some(Value::Object(existing)), Value::String(_)) => existing.insert("severity".into(), v.clone()),
            _ => target.insert(k.clone(), v.clone()),
        }
    }
}

/// `resolveAndMergeNestedStyleguideConfig`.
fn resolve(styleguide: Option<&Map>, config: &Path, stack: &mut Vec<PathBuf>) -> Result<Merged, String> {
    let abs = std::path::absolute(config).unwrap_or_else(|_| config.to_path_buf());
    if stack.contains(&abs) {
        return Err(format!("Circular dependency in config file: \"{}\"", config.display()));
    }
    stack.push(abs);
    let mut merged = Merged::default();
    let empty = Map::new();
    let styleguide = styleguide.unwrap_or(&empty);
    let extends = match styleguide.get("extends") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    };
    for item in &extends {
        let Value::String(name) = item else {
            return Err("Error configuration format not detected in extends value must contain strings".into());
        };
        if name.starts_with("http://") || name.starts_with("https://") {
            return Err(format!("Failed to load \"{name}\": remote Redocly configs are not supported by spectype"));
        }
        if Path::new(name).extension().is_none() {
            merged.add(&preset(name)?);
            continue;
        }
        let file = config.parent().unwrap_or_else(|| Path::new("")).join(name);
        let loaded = read_config(&file)
            .and_then(|raw| {
                styleguide_of(&raw)?
                    .ok_or_else(|| format!("Styleguide configuration format not detected: \"{}\"", file.display()))
            })
            .map_err(|e| format!("Failed to load \"{}\": {e}", file.display()))?;
        let nested = resolve(Some(&loaded), &file, stack)?;
        let mut flat = Map::new();
        flat.insert("rules".into(), Value::Object(nested.rules));
        flat.insert("oas3_0Rules".into(), Value::Object(nested.oas3_0));
        flat.insert("oas3_1Rules".into(), Value::Object(nested.oas3_1));
        merged.add(&flat);
        merged.plugins |= nested.plugins;
    }
    merged.add(styleguide);
    stack.pop();
    Ok(merged)
}

fn preset(name: &str) -> Result<Map, String> {
    if let Some((plugin, _)) = name.split_once('/') {
        return Err(format!("Invalid config {name}: plugin {plugin} is not included."));
    }
    let presets = parse_document(PRESETS, Format::Json).map_err(|e| e.to_string())?;
    match presets.get(name) {
        Some(Value::Object(m)) => Ok(m.clone()),
        _ => Err(format!("Invalid config {name}: there is no such built-in config.")),
    }
}

/// `new URL(path, configFileUrl)`: relative to the config file's directory.
pub fn relative_to_config(file: Option<&Path>, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || Path::new(path).is_absolute() {
        return path.to_owned();
    }
    let dir = file.and_then(Path::parent).unwrap_or_else(|| Path::new(""));
    dir.join(path).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(yaml: &str) -> LintConfig {
        let dir = std::env::temp_dir().join(format!("spectype-config-{}-{}", std::process::id(), yaml.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("redocly.yaml");
        std::fs::write(&path, yaml).unwrap();
        let lint = load(Some(&path.to_string_lossy())).unwrap().lint;
        let _ = std::fs::remove_dir_all(&dir);
        lint
    }

    fn severity(c: &LintConfig, rule: &str) -> Option<String> {
        c.oas3_0.get(rule).map(|v| match v {
            Value::Object(m) => m.get("severity").map(Value::js_string).unwrap_or_default(),
            v => v.js_string(),
        })
    }

    #[test]
    fn extends_and_overrides() {
        let c = config(
            "extends: [minimal]\nrules:\n  no-server-trailing-slash: warn\n  no-invalid-media-type-examples: error\n  spec: warn\n",
        );
        assert_eq!(severity(&c, "no-server-trailing-slash").as_deref(), Some("warn"));
        assert_eq!(severity(&c, "no-invalid-media-type-examples").as_deref(), Some("error"));
        assert_eq!(severity(&c, "struct").as_deref(), Some("warn"));
        assert_eq!(severity(&c, "operation-summary").as_deref(), Some("warn"));
    }

    #[test]
    fn no_extends_means_only_listed_rules() {
        let c = config("rules:\n  operation-summary: error\n");
        assert_eq!(c.oas3_0.len(), 1);
        assert!(config("apis: {}\n").is_empty());
    }
}

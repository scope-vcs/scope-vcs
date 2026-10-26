use crate::{
    limits,
    paths::{file_name, join_relative, parent_directory, resolve_lexically},
    snapshot::{Inventory, absolute_snapshot_path},
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

pub(crate) type ConfigGroups = BTreeMap<Option<String>, Vec<String>>;

pub(crate) struct ResolutionConfig {
    pub(crate) config_file: PathBuf,
    pub(crate) path_patterns: Vec<String>,
    pub(crate) module_suffixes: Vec<String>,
}

pub(crate) fn group_sources_by_config(
    sources: &[String],
    configs: &[String],
) -> Result<ConfigGroups, String> {
    let mut by_directory: BTreeMap<&str, &str> = BTreeMap::new();
    for config in configs {
        let directory = parent_directory(config);
        let preferred = by_directory.get(directory).is_none_or(|current| {
            (config_rank(config), config.as_str()) < (config_rank(current), current)
        });
        if preferred {
            by_directory.insert(directory, config);
        }
    }

    let mut groups = ConfigGroups::new();
    for source in sources {
        let mut directory = parent_directory(source);
        let selected = loop {
            if let Some(config) = by_directory.get(directory) {
                break Some((*config).to_owned());
            }
            if directory == "." {
                break None;
            }
            directory = parent_directory(directory);
        };
        groups.entry(selected).or_default().push(source.clone());
    }

    if groups.keys().flatten().count() > limits::MAX_CONFIGS {
        return Err(format!(
            "TypeScript config limit exceeded ({})",
            limits::MAX_CONFIGS
        ));
    }
    Ok(groups)
}

fn config_rank(path: &str) -> u8 {
    match file_name(path) {
        "tsconfig.json" => 0,
        "jsconfig.json" => 1,
        name if name.starts_with("tsconfig.") => 2,
        _ => 3,
    }
}

pub(crate) struct ConfigLoader<'a> {
    inventory: &'a Inventory,
    visiting: Vec<String>,
    loaded: HashMap<String, CompilerOptions>,
}

impl<'a> ConfigLoader<'a> {
    pub(crate) fn new(inventory: &'a Inventory) -> Self {
        Self {
            inventory,
            visiting: Vec::new(),
            loaded: HashMap::new(),
        }
    }

    pub(crate) fn resolution_config(
        &mut self,
        config_path: &str,
    ) -> Result<ResolutionConfig, String> {
        let options = self
            .load(config_path)
            .map_err(|reason| format!("invalid TypeScript config: {reason}"))?;
        Ok(ResolutionConfig {
            config_file: self.inventory.root.join(config_path),
            path_patterns: options
                .paths
                .map(|paths| paths.keys().cloned().collect())
                .unwrap_or_default(),
            module_suffixes: options.module_suffixes.unwrap_or_default(),
        })
    }

    fn load(&mut self, config_path: &str) -> Result<CompilerOptions, String> {
        if let Some(options) = self.loaded.get(config_path) {
            return Ok(options.clone());
        }
        if self.visiting.iter().any(|visiting| visiting == config_path) {
            return Err(format!("extended config cycle at {config_path}"));
        }
        if self.visiting.len() >= limits::MAX_CONFIGS {
            return Err(format!(
                "extended config depth limit exceeded ({})",
                limits::MAX_CONFIGS
            ));
        }
        self.visiting.push(config_path.to_owned());
        let loaded = self.load_uncached(config_path);
        self.visiting.pop();
        let options = loaded?;
        self.loaded.insert(config_path.to_owned(), options.clone());
        Ok(options)
    }
}

#[derive(Clone, Default)]
struct CompilerOptions {
    base_url: Option<PathBuf>,
    paths_directory: Option<PathBuf>,
    paths: Option<Map<String, Value>>,
    module_suffixes: Option<Vec<String>>,
}

impl CompilerOptions {
    fn extended_by(self, child: Self) -> Self {
        Self {
            base_url: child.base_url.or(self.base_url),
            paths_directory: child.paths_directory.or(self.paths_directory),
            paths: child.paths.or(self.paths),
            module_suffixes: child.module_suffixes.or(self.module_suffixes),
        }
    }
}

impl ConfigLoader<'_> {
    fn load_uncached(&mut self, config_path: &str) -> Result<CompilerOptions, String> {
        let inventory = self.inventory;
        let root = &inventory.root;
        let config_file = inventory.absolute(config_path)?;
        let config =
            read_config(&config_file).ok_or_else(|| format!("malformed JSONC at {config_path}"))?;
        let own = match config.get("compilerOptions") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(options)) => options.clone(),
            Some(_) => return Err("compilerOptions must be an object".into()),
        };
        validate_compiler_options(&own)?;

        let mut options = CompilerOptions::default();
        for extended in extends_values(&config)? {
            let extended_path = self.resolve_extended(config_path, &extended)?;
            options = options.extended_by(self.load(&extended_path)?);
        }

        let config_directory = config_file
            .parent()
            .map_or_else(|| root.clone(), Path::to_path_buf);
        if let Some(base_url) = own.get("baseUrl").and_then(Value::as_str) {
            let base_url = resolve_lexically(&config_directory, base_url);
            absolute_snapshot_path(root, &base_url)?;
            options.base_url = Some(base_url);
        }
        let own_paths = own.get("paths").and_then(Value::as_object).cloned();
        if own_paths.is_some() {
            options.paths_directory = Some(config_directory.clone());
        }
        for directory in string_array(&own, "rootDirs").unwrap_or_default() {
            absolute_snapshot_path(root, resolve_lexically(&config_directory, directory))?;
        }
        if let Some(module_suffixes) = string_array(&own, "moduleSuffixes") {
            options.module_suffixes = Some(module_suffixes);
        }
        if let Some(paths) = own_paths {
            options.paths = Some(paths);
        }

        let paths_base = options.base_url.as_ref().unwrap_or(&config_directory);
        let paths_directory = options
            .paths_directory
            .as_ref()
            .unwrap_or(&config_directory)
            .to_string_lossy()
            .into_owned();
        for target in options
            .paths
            .iter()
            .flat_map(Map::values)
            .flat_map(path_targets)
        {
            let target = target
                .replace("${configDir}", &paths_directory)
                .replace('*', "segment");
            absolute_snapshot_path(root, resolve_lexically(paths_base, target))?;
        }
        Ok(options)
    }

    fn resolve_extended(&self, current: &str, requested: &str) -> Result<String, String> {
        if requested.starts_with('/') {
            return Err(format!("extended config escapes snapshot: {requested}"));
        }
        let base = join_relative(parent_directory(current), requested);
        if base == ".." || base.starts_with("../") {
            return Err(format!("extended config escapes snapshot: {requested}"));
        }
        let nested = join_relative(&base, "tsconfig.json");
        let candidates = [base.clone(), format!("{base}.json"), nested];
        if let Some(found) = candidates
            .into_iter()
            .find(|candidate| self.inventory.all_files.contains(candidate))
        {
            return Ok(found);
        }
        if !requested.starts_with('.') {
            return Err(format!(
                "package-based extended config is unavailable: {requested}"
            ));
        }
        Err(format!("extended config is missing: {requested}"))
    }
}

fn read_config(path: &Path) -> Option<Map<String, Value>> {
    let mut text = fs::read_to_string(path).ok()?;
    json_strip_comments::strip(&mut text).ok()?;
    match serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()? {
        Value::Object(config) => Some(config),
        _ => None,
    }
}

fn extends_values(config: &Map<String, Value>) -> Result<Vec<String>, String> {
    match config.get("extends") {
        None => Ok(Vec::new()),
        Some(Value::String(value)) => Ok(vec![value.clone()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(extends_error),
        Some(_) => Err(extends_error()),
    }
}

fn extends_error() -> String {
    "extends must be a string or an array of strings".into()
}

fn validate_compiler_options(options: &Map<String, Value>) -> Result<(), String> {
    if options
        .get("baseUrl")
        .is_some_and(|value| !value.is_string())
    {
        return Err("baseUrl must be a string".into());
    }
    if let Some(paths) = options.get("paths") {
        let paths = paths.as_object().ok_or("paths must be an object")?;
        for (pattern, targets) in paths {
            let valid = targets
                .as_array()
                .is_some_and(|targets| !targets.is_empty() && targets.iter().all(Value::is_string));
            if !valid {
                return Err(format!(
                    "paths mapping must contain string targets: {pattern}"
                ));
            }
        }
    }
    for option in ["rootDirs", "moduleSuffixes"] {
        let valid = options.get(option).is_none_or(|value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
        });
        if !valid {
            return Err(format!("{option} must be an array of strings"));
        }
    }
    if string_array(options, "moduleSuffixes")
        .is_some_and(|suffixes| suffixes.iter().any(|suffix| suffix.contains(['/', '\\'])))
    {
        return Err("moduleSuffixes cannot contain path separators".into());
    }
    Ok(())
}

fn string_array(options: &Map<String, Value>, key: &str) -> Option<Vec<String>> {
    options.get(key)?.as_array().map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}

fn path_targets(targets: &Value) -> impl Iterator<Item = &str> {
    targets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

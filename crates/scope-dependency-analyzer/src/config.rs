use crate::{
    jsonc, limits,
    paths::{file_name, join_relative, parent_directory, resolve_lexically},
    snapshot::{Inventory, absolute_snapshot_path},
};
use serde_json::{Map, Value};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

pub(crate) type ConfigGroups = BTreeMap<Option<String>, Vec<String>>;

pub(crate) struct ResolutionConfig {
    pub(crate) path_patterns: Vec<String>,
    pub(crate) path_aliases: Vec<PathAlias>,
    pub(crate) module_directories: Vec<PathBuf>,
    pub(crate) root_dirs: Vec<PathBuf>,
    pub(crate) module_suffixes: Vec<String>,
}

impl ResolutionConfig {
    fn new(options: CompilerOptions, config_directory: &Path) -> Self {
        let (module_patterns, mut path_aliases): (Vec<_>, Vec<_>) =
            path_aliases(&options, config_directory)
                .into_iter()
                .partition(|alias| alias.pattern == "*");
        let mut module_directories = module_patterns
            .iter()
            .flat_map(|alias| &alias.targets)
            .filter_map(|target| {
                target
                    .strip_suffix("/*")
                    .or_else(|| target.strip_suffix("\\*"))
                    .map(PathBuf::from)
            })
            .collect::<Vec<_>>();
        if let Some(base_url) = options.base_url
            && !module_directories.contains(&base_url)
        {
            module_directories.push(base_url);
        }
        path_aliases.sort_by_key(|alias| Reverse(wildcard_prefix_length(&alias.pattern)));
        Self {
            path_patterns: options
                .paths
                .into_iter()
                .flatten()
                .map(|(pattern, _)| pattern)
                .collect(),
            path_aliases,
            module_directories,
            root_dirs: options.root_dirs.unwrap_or_default(),
            module_suffixes: options.module_suffixes.unwrap_or_default(),
        }
    }

    pub(crate) fn claims(&self, specifier: &str) -> bool {
        self.path_patterns
            .iter()
            .any(|pattern| matches_path_pattern(pattern, specifier))
    }

    pub(crate) fn alias_targets(&self, specifier: &str) -> Option<Vec<String>> {
        self.path_aliases
            .iter()
            .find_map(|alias| alias.substitute(specifier))
    }

    pub(crate) fn root_dir_candidates(&self, candidate: &Path) -> Vec<PathBuf> {
        let Some(original) = self
            .root_dirs
            .iter()
            .filter(|directory| candidate.starts_with(directory))
            .fold(None, |longest: Option<&PathBuf>, directory| match longest {
                Some(longest) if longest.as_os_str().len() >= directory.as_os_str().len() => {
                    Some(longest)
                }
                _ => Some(directory),
            })
        else {
            return Vec::new();
        };
        let Ok(relative) = candidate.strip_prefix(original) else {
            return Vec::new();
        };
        self.root_dirs
            .iter()
            .filter(|directory| *directory != original)
            .map(|directory| directory.join(relative))
            .collect()
    }
}

impl PathAlias {
    fn substitute(&self, specifier: &str) -> Option<Vec<String>> {
        let pattern = self.pattern.as_str();
        if let Some(remaining) = specifier.strip_prefix(pattern)
            && (remaining.is_empty() || remaining.starts_with('/'))
        {
            return Some(
                self.targets
                    .iter()
                    .map(|target| format!("{target}{remaining}"))
                    .collect(),
            );
        }
        let (prefix, suffix) = pattern.split_once('*')?;
        if suffix.contains('*') || !specifier.starts_with(prefix) || !specifier.ends_with(suffix) {
            return None;
        }
        let captured = specifier
            .get(prefix.len()..specifier.len() - suffix.len())
            .unwrap_or_default();
        Some(
            self.targets
                .iter()
                .map(|target| target.replacen('*', captured, 1))
                .collect(),
        )
    }
}

fn matches_path_pattern(pattern: &str, specifier: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    let [first, middle @ .., last] = parts.as_slice() else {
        return pattern == specifier;
    };
    let Some(mut rest) = specifier.strip_prefix(first) else {
        return false;
    };
    for part in middle {
        let Some(index) = rest.find(part) else {
            return false;
        };
        rest = &rest[index + part.len()..];
    }
    rest.ends_with(last)
}

pub(crate) struct PathAlias {
    pub(crate) pattern: String,
    pub(crate) targets: Vec<String>,
}

type PathMappings = Vec<(String, Vec<String>)>;

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
        Ok(ResolutionConfig::new(
            options,
            &config_directory(&self.inventory.root, config_path),
        ))
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
    paths: Option<PathMappings>,
    root_dirs: Option<Vec<PathBuf>>,
    module_suffixes: Option<Vec<String>>,
}

impl CompilerOptions {
    fn extended_by(self, child: Self) -> Self {
        Self {
            base_url: child.base_url.or(self.base_url),
            paths: child.paths.or(self.paths),
            root_dirs: child.root_dirs.or(self.root_dirs),
            module_suffixes: child.module_suffixes.or(self.module_suffixes),
        }
    }
}

fn config_directory(root: &Path, config_path: &str) -> PathBuf {
    root.join(parent_directory(config_path))
}

fn path_aliases(options: &CompilerOptions, config_directory: &Path) -> Vec<PathAlias> {
    let base = options.base_url.as_deref().unwrap_or(config_directory);
    let config_directory = config_directory.to_string_lossy();
    options
        .paths
        .iter()
        .flatten()
        .map(|(pattern, targets)| PathAlias {
            pattern: pattern.clone(),
            targets: targets
                .iter()
                .map(|target| {
                    resolve_lexically(base, target.replace("${configDir}", &config_directory))
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
        })
        .collect()
}

fn wildcard_prefix_length(pattern: &str) -> usize {
    pattern.find('*').unwrap_or(pattern.len())
}

impl ConfigLoader<'_> {
    fn load_uncached(&mut self, config_path: &str) -> Result<CompilerOptions, String> {
        let inventory = self.inventory;
        let root = &inventory.root;
        let config_file = inventory.absolute(config_path)?;
        let config =
            read_config(&config_file).ok_or_else(|| format!("malformed JSONC at {config_path}"))?;
        let own = match config.values.get("compilerOptions") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(options)) => options.clone(),
            Some(_) => return Err("compilerOptions must be an object".into()),
        };
        validate_compiler_options(&own)?;

        let mut options = CompilerOptions::default();
        for extended in extends_values(&config.values)? {
            let extended_path = self.resolve_extended(config_path, &extended)?;
            options = options.extended_by(self.load(&extended_path)?);
        }

        let config_directory = config_directory(root, config_path);
        if let Some(base_url) = own.get("baseUrl").and_then(Value::as_str)
            && !base_url.is_empty()
        {
            let base_url = resolve_lexically(&config_directory, base_url);
            absolute_snapshot_path(root, &base_url)?;
            options.base_url = Some(base_url);
        }
        if let Some(root_dirs) = string_array(&own, "rootDirs") {
            let root_dirs = root_dirs
                .into_iter()
                .map(|directory| {
                    absolute_snapshot_path(root, resolve_lexically(&config_directory, directory))
                })
                .collect::<Result<Vec<_>, _>>()?;
            options.root_dirs = Some(root_dirs);
        }
        if let Some(module_suffixes) = string_array(&own, "moduleSuffixes") {
            options.module_suffixes = Some(module_suffixes);
        }
        if own.contains_key("paths") {
            options.paths = Some(config.path_mappings);
        }

        for alias in path_aliases(&options, &config_directory) {
            for target in alias.targets {
                absolute_snapshot_path(root, target.replace('*', "segment"))?;
            }
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
        let mut candidates = Vec::with_capacity(3);
        if !requested.contains(".json") {
            candidates.push(format!("{base}.json"));
        }
        candidates.extend([base.clone(), nested]);
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

struct ConfigDocument {
    values: Map<String, Value>,
    path_mappings: PathMappings,
}

fn read_config(path: &Path) -> Option<ConfigDocument> {
    let mut text = fs::read_to_string(path).ok()?;
    if jsonc::has_hash_comment(&text) {
        return None;
    }
    json_strip_comments::strip(&mut text).ok()?;
    let text = text.trim_start_matches('\u{feff}');
    let Value::Object(values) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let path_mappings = match values
        .get("compilerOptions")
        .and_then(|options| options.get("paths"))
    {
        Some(_) => jsonc::ordered_path_mappings(text)?,
        None => Vec::new(),
    };
    Some(ConfigDocument {
        values,
        path_mappings,
    })
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

use crate::manifest::{load_manifest, validate_manifest};
use std::os::unix::fs::symlink;
use anyhow::{Context, Result, bail};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use url::Url;

#[derive(Serialize, Deserialize, Default)]
pub struct MplugConfig {
    pub enabled_plugins: HashSet<String>,
}

pub fn get_config_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    path.push("mplug");

    if let Err(e) = fs::create_dir_all(path.join("plugins")) {
        eprintln!("mplug: warning: could not create plugins directory: {e}");
    }
    path
}

fn get_config_path() -> PathBuf {
    get_config_dir().join("mplug.toml")
}

pub fn load_config() -> MplugConfig {
    let path = get_config_path();
    if let Ok(content) = fs::read_to_string(&path) {
        toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("mplug: warning: config file has invalid TOML, using defaults: {e}");
            MplugConfig::default()
        })
    } else {
        MplugConfig::default()
    }
}

pub fn save_config(config: &MplugConfig) -> Result<()> {
    let path = get_config_path();
    let content = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(&path, &content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

fn plugin_installed(plugins_dir: &std::path::Path, plugin: &str) -> bool {
    plugins_dir.join(plugin).is_dir()
        || plugins_dir.join(plugin).with_extension("lua").is_file()
        || plugins_dir
            .join(plugin)
            .with_extension("lua")
            .is_symlink()
}

pub fn enable_plugin(plugin: &str) -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");
    if !plugin_installed(&plugins_dir, plugin) {
        bail!(
            "Plugin '{}' is not installed — use `mplug list` to see installed plugins",
            plugin
        )
    }
    let mut config = load_config();
    if config.enabled_plugins.insert(plugin.to_string()) {
        save_config(&config)?;
        println!("{} {} enabled", style("✔").green().bold(), plugin);
    } else {
        println!(
            "{} '{}' is already enabled",
            style("!").yellow().bold(),
            plugin
        );
    }
    Ok(())
}

pub fn disable_plugin(plugin: &str) -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");
    if !plugin_installed(&plugins_dir, plugin) {
        bail!(
            "Plugin '{}' is not installed — use `mplug list` to see installed plugins",
            plugin
        )
    }
    let mut config = load_config();
    if config.enabled_plugins.remove(plugin) {
        save_config(&config)?;
        println!("{} {} disabled", style("✔").green().bold(), plugin);
    } else {
        println!("{} '{}' is not enabled", style("!").yellow().bold(), plugin);
    }
    Ok(())
}

// Returns the collection directory name if `link` is a symlink pointing into a
// collection repo inside `plugins_dir`, otherwise None.
fn collection_source(link: &std::path::Path, plugins_dir: &std::path::Path) -> Option<String> {
    let target = fs::read_link(link).ok()?;
    // Symlinks are stored as absolute paths.
    let col_dir = target.parent()?;
    if !col_dir.starts_with(plugins_dir) {
        return None;
    }
    let col_name = col_dir.file_name()?.to_str()?.to_string();
    if let Ok(manifest) = load_manifest(col_dir) {
        if manifest.collection.is_some() {
            return Some(col_name);
        }
    }
    None
}

pub fn list_plugins() -> Result<()> {
    let config = load_config();
    let plugins_dir = get_config_dir().join("plugins");

    let entries = fs::read_dir(&plugins_dir)
        .with_context(|| format!("Cannot read plugins directory: {}", plugins_dir.display()))?;

    // (sort_key, display_name, enabled, via_collection, is_collection_dir)
    enum Row {
        Plugin { name: String, enabled: bool, via: Option<String> },
        Collection { name: String, member_count: usize },
    }

    let mut rows: Vec<Row> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("lua") {
            let Some(name) = path.file_stem().and_then(|n| n.to_str()).map(str::to_string)
            else {
                continue;
            };
            let via = if path.is_symlink() {
                collection_source(&path, &plugins_dir)
            } else {
                None
            };
            rows.push(Row::Plugin {
                enabled: config.enabled_plugins.contains(&name),
                name,
                via,
            });
        } else if path.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string)
            else {
                continue;
            };
            // If this dir is a collection repo, show it as such (not enable-able directly).
            if let Ok(manifest) = load_manifest(&path) {
                if let Some(col) = &manifest.collection {
                    rows.push(Row::Collection {
                        name,
                        member_count: col.plugins.len(),
                    });
                    continue;
                }
            }
            rows.push(Row::Plugin {
                enabled: config.enabled_plugins.contains(&name),
                name,
                via: None,
            });
        }
    }

    if rows.is_empty() {
        println!("{} No plugins installed", style("!").yellow().bold());
        println!("  {} add one with: mplug add <repo>", style("→").dim());
        return Ok(());
    }

    rows.sort_by_key(|r| match r {
        Row::Plugin { name, .. } => name.clone(),
        Row::Collection { name, .. } => name.clone(),
    });

    let plugin_count = rows
        .iter()
        .filter(|r| matches!(r, Row::Plugin { .. }))
        .count();
    println!("Installed plugins ({})", plugin_count);

    for row in &rows {
        match row {
            Row::Plugin { name, enabled, via: None } => {
                if *enabled {
                    println!("  {} {}  {}", style("→").cyan(), name, style("enabled").green());
                } else {
                    println!("  {} {}  {}", style("→").dim(), name, style("disabled").dim());
                }
            }
            Row::Plugin { name, enabled, via: Some(col) } => {
                if *enabled {
                    println!(
                        "  {} {}  {}  {}",
                        style("→").cyan(),
                        name,
                        style("enabled").green(),
                        style(format!("(via {col})")).dim()
                    );
                } else {
                    println!(
                        "  {} {}  {}  {}",
                        style("→").dim(),
                        name,
                        style("disabled").dim(),
                        style(format!("(via {col})")).dim()
                    );
                }
            }
            Row::Collection { name, member_count } => {
                println!(
                    "  {} {}  {}",
                    style("→").yellow(),
                    name,
                    style(format!("collection ({member_count} plugins — update with: mplug update {name})")).dim()
                );
            }
        }
    }
    Ok(())
}

pub fn add_plugin(repo: &str) -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");

    let dir_name = match Url::parse(repo) {
        Ok(url) => url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or("unknown_plugin")
            .trim_end_matches(".git")
            .to_string(),
        Err(_) => {
            bail!("Invalid repository URL: {repo}")
        }
    };

    let target_path = plugins_dir.join(&dir_name);

    if target_path.exists() {
        bail!(
            "Plugin '{}' is already installed — use `mplug update {}` to update it",
            dir_name,
            dir_name
        )
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
    pb.set_message(format!("Cloning '{}'...", dir_name));
    pb.enable_steady_tick(Duration::from_millis(80));

    let output = Command::new("git")
        .args(["clone", "--quiet", repo])
        .arg(&target_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    pb.finish_and_clear();

    match output {
        Ok(out) if out.status.success() => {
            match load_manifest(&target_path) {
                Err(err) => {
                    let _ = fs::remove_dir_all(&target_path);
                    bail!("Plugin '{}' has no valid mplug.toml: {}", dir_name, err)
                }
                Ok(manifest) => {
                    if let Err(err) = validate_manifest(&manifest) {
                        let _ = fs::remove_dir_all(&target_path);
                        bail!("Plugin '{}' has no valid mplug.toml: {}", dir_name, err)
                    }

                    if let Some(col) = &manifest.collection {
                        // Collection: create a symlink per member in plugins_dir
                        let mut linked = Vec::new();
                        let mut errors = Vec::new();
                        for plugin in &col.plugins {
                            let src = target_path.join(format!("{}.lua", plugin));
                            let link = plugins_dir.join(format!("{}.lua", plugin));
                            if link.exists() || link.is_symlink() {
                                errors.push(format!(
                                    "'{}' already exists — skipping",
                                    plugin
                                ));
                            } else {
                                match symlink(&src, &link) {
                                    Ok(()) => linked.push(plugin.clone()),
                                    Err(e) => errors.push(format!("'{}': {}", plugin, e)),
                                }
                            }
                        }
                        println!(
                            "{} Added collection: {}",
                            style("✔").green().bold(),
                            dir_name
                        );
                        for name in &linked {
                            println!(
                                "  {} mplug enable {}",
                                style("→").dim(),
                                name
                            );
                        }
                        for err in &errors {
                            println!("  {} {}", style("!").yellow().bold(), err);
                        }
                    } else {
                        println!("{} Added: {}", style("✔").green().bold(), dir_name);
                        println!(
                            "  {} enable it with: mplug enable {}",
                            style("→").dim(),
                            dir_name
                        );
                    }
                }
            }
        }
        Ok(out) => bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => bail!("Failed to spawn git: {e} — is git installed?"),
    }
    Ok(())
}

pub fn remove_plugin(name: &str) -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");
    let plugin_dir = plugins_dir.join(name);
    let plugin_file = plugins_dir.join(name).with_extension("lua");

    if !plugin_dir.exists() && !plugin_file.exists() && !plugin_file.is_symlink() {
        bail!(
            "Plugin '{}' is not installed — use `mplug list` to see installed plugins",
            name
        )
    }

    let mut config = load_config();

    if plugin_dir.is_dir() {
        // If this is a collection repo, remove each member's symlink and enabled entry.
        if let Ok(manifest) = load_manifest(&plugin_dir) {
            if let Some(col) = &manifest.collection {
                for member in &col.plugins {
                    let link = plugins_dir.join(format!("{}.lua", member));
                    if link.is_symlink() || link.exists() {
                        fs::remove_file(&link).with_context(|| {
                            format!("Failed to remove symlink for '{}'", member)
                        })?;
                    }
                    config.enabled_plugins.remove(member);
                }
            }
        }
        config.enabled_plugins.remove(name);
        fs::remove_dir_all(&plugin_dir)
            .with_context(|| format!("Failed to remove {}", plugin_dir.display()))?;
    } else {
        config.enabled_plugins.remove(name);
        fs::remove_file(&plugin_file)
            .with_context(|| format!("Failed to remove {}", plugin_file.display()))?;
    }

    save_config(&config)?;
    println!("{} Removed: {}", style("✔").green().bold(), name);
    Ok(())
}

pub fn outdated_plugins() -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");

    let entries = match fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => {
            bail!("No plugins directory found at {}", plugins_dir.display())
        }
    };

    let mut checked = false;
    let mut has_outdated = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let _ = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["fetch", "--quiet"])
            .status();

        let output = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["rev-list", "--count", "HEAD..@{u}"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let behind = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let count: u32 = behind.parse().unwrap_or(0);
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    checked = true;
                    if count > 0 {
                        has_outdated = true;
                        println!(
                            "  {} {}  ({} commit(s) behind)",
                            style("✖").red().bold(),
                            name,
                            count
                        );
                    } else {
                        println!("  {} {}", style("✔").green().bold(), name);
                    }
                }
            }
        }
    }

    if !checked {
        println!(
            "{} No git-managed plugins found",
            style("!").yellow().bold()
        );
    }
    if has_outdated {
        println!("  {} run `mplug update <name>` to update", style("→").dim());
    }
    Ok(())
}

pub fn update_plugin(name: &str) -> Result<()> {
    let plugins_dir = get_config_dir().join("plugins");
    let plugin_path = plugins_dir.join(name);

    if !plugin_path.exists() {
        bail!(
            "Plugin '{}' is not installed — use `mplug list` to see installed plugins",
            name
        )
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
    pb.set_message(format!("Updating '{}'...", name));
    pb.enable_steady_tick(Duration::from_millis(80));

    let output = Command::new("git")
        .args(["-C"])
        .arg(&plugin_path)
        .args(["pull", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    pb.finish_and_clear();

    match output {
        Ok(out) if out.status.success() => {
            println!("{} Updated: {}", style("✔").green().bold(), name)
        }
        Ok(out) => bail!(
            "git pull failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => bail!("Failed to spawn git: {e} — is git installed?"),
    }
    Ok(())
}

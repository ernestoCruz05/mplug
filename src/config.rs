use crate::manifest::{load_manifest, validate_manifest};
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
    plugins_dir.join(plugin).is_dir() || plugins_dir.join(plugin).with_extension("lua").is_file()
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

pub fn list_plugins() -> Result<()> {
    let config = load_config();
    let plugins_dir = get_config_dir().join("plugins");

    let entries = fs::read_dir(&plugins_dir)
        .with_context(|| format!("Cannot read plugins directory: {}", plugins_dir.display()))?;

    let mut rows: Vec<(String, bool)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("lua") {
            if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                rows.push((name.to_string(), config.enabled_plugins.contains(name)));
            }
        } else if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                rows.push((name.to_string(), config.enabled_plugins.contains(name)));
            }
        }
    }

    if rows.is_empty() {
        println!("{} No plugins installed", style("!").yellow().bold());
        println!("  {} add one with: mplug add <repo>", style("→").dim());
        return Ok(());
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));

    println!("Installed plugins ({})", rows.len());
    for (name, enabled) in &rows {
        if *enabled {
            println!(
                "  {} {}  {}",
                style("→").cyan(),
                name,
                style("enabled").green()
            );
        } else {
            println!(
                "  {} {}  {}",
                style("→").dim(),
                name,
                style("disabled").dim()
            );
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
                }
            }
            println!("{} Added: {}", style("✔").green().bold(), dir_name);
            println!(
                "  {} enable it with: mplug enable {}",
                style("→").dim(),
                dir_name
            );
        }
        Ok(out) => bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => bail!("Failed to spawn git: {e} — is git installed?"),
    }
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

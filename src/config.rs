use crate::manifest::{load_manifest, validate_manifest};
use anyhow::{Context, Result, bail};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::symlink;
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
        || plugins_dir.join(plugin).with_extension("lua").is_symlink()
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
        if let Some(members) = collection_members(&plugins_dir, plugin) {
            println!(
                "  {} collection — enables {} plugins:",
                style("→").dim(),
                members.len()
            );
            for m in &members {
                println!("    {} {}", style("•").dim(), m);
            }
        }
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
        if let Some(members) = collection_members(&plugins_dir, plugin) {
            let still = still_enabled_members(&members, &config.enabled_plugins);
            if still.is_empty() {
                println!(
                    "  {} all of its member plugins are now disabled",
                    style("→").dim()
                );
            } else {
                println!(
                    "  {} still enabled individually: {}",
                    style("!").yellow().bold(),
                    still.join(", ")
                );
            }
        }
    } else {
        println!("{} '{}' is not enabled", style("!").yellow().bold(), plugin);
    }
    Ok(())
}

fn collection_members(plugins_dir: &std::path::Path, name: &str) -> Option<Vec<String>> {
    let dir = plugins_dir.join(name);
    if !dir.is_dir() {
        return None;
    }
    load_manifest(&dir).ok()?.collection.map(|c| c.plugins)
}

fn still_enabled_members(members: &[String], enabled: &HashSet<String>) -> Vec<String> {
    members
        .iter()
        .filter(|m| enabled.contains(*m))
        .cloned()
        .collect()
}

fn status_label(status: &PluginStatus) -> console::StyledObject<&'static str> {
    match status {
        PluginStatus::Enabled => style("enabled").green(),
        PluginStatus::Partial => style("partial").yellow(),
        PluginStatus::Disabled => style("disabled").dim(),
    }
}

fn enabled_label(enabled: bool) -> console::StyledObject<&'static str> {
    if enabled {
        style("enabled").green()
    } else {
        style("disabled").dim()
    }
}

pub fn list_plugins() -> Result<()> {
    let config = load_config();
    let plugins_dir = get_config_dir().join("plugins");

    fs::read_dir(&plugins_dir)
        .with_context(|| format!("Cannot read plugins directory: {}", plugins_dir.display()))?;

    let listing = build_listing(&plugins_dir, &config.enabled_plugins);

    if listing.collections.is_empty() && listing.standalones.is_empty() {
        println!("{} No plugins installed", style("!").yellow().bold());
        println!("  {} add one with: mplug add <repo>", style("→").dim());
        return Ok(());
    }

    let plugin_count: usize = listing.standalones.len()
        + listing
            .collections
            .iter()
            .map(|c| c.members.len())
            .sum::<usize>();
    println!("Installed plugins ({plugin_count})");

    for col in &listing.collections {
        let bullet = match col.status {
            PluginStatus::Disabled => style("→").dim(),
            _ => style("→").cyan(),
        };
        println!(
            "  {} {}  {}  {}",
            bullet,
            col.name,
            status_label(&col.status),
            style(format!(
                "collection ({} plugins — update with: mplug update {})",
                col.members.len(),
                col.name
            ))
            .dim()
        );
        for (i, member) in col.members.iter().enumerate() {
            let branch = if i + 1 == col.members.len() {
                "└─"
            } else {
                "├─"
            };
            println!(
                "    {} {}  {}",
                style(branch).dim(),
                member.name,
                enabled_label(member.enabled)
            );
        }
    }

    for plugin in &listing.standalones {
        let bullet = if plugin.enabled {
            style("→").cyan()
        } else {
            style("→").dim()
        };
        println!(
            "  {} {}  {}",
            bullet,
            plugin.name,
            enabled_label(plugin.enabled)
        );
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
        Ok(out) if out.status.success() => match load_manifest(&target_path) {
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
                    let mut linked = Vec::new();
                    let mut errors = Vec::new();
                    for plugin in &col.plugins {
                        let src = target_path.join(format!("{}.lua", plugin));
                        let link = plugins_dir.join(format!("{}.lua", plugin));
                        if link.exists() || link.is_symlink() {
                            errors.push(format!("'{}' already exists — skipping", plugin));
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
                        println!("  {} mplug enable {}", style("→").dim(), name);
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
        },
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

#[derive(Debug, PartialEq)]
pub enum PluginStatus {
    Enabled,
    Partial,
    Disabled,
}

#[derive(Debug, PartialEq)]
pub struct MemberView {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, PartialEq)]
pub struct CollectionView {
    pub name: String,
    pub status: PluginStatus,
    pub members: Vec<MemberView>,
}

#[derive(Debug, PartialEq)]
pub struct StandaloneView {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, PartialEq, Default)]
pub struct Listing {
    pub collections: Vec<CollectionView>,
    pub standalones: Vec<StandaloneView>,
}

pub fn resolve_load_targets(plugins_dir: &std::path::Path, name: &str) -> Vec<(String, PathBuf)> {
    let file = plugins_dir.join(name).with_extension("lua");
    if file.exists() {
        return vec![(name.to_string(), file)];
    }

    let dir = plugins_dir.join(name);
    if dir.is_dir() {
        if let Ok(manifest) = load_manifest(&dir) {
            if let Some(ep) = manifest
                .entry_point
                .as_deref()
                .map(str::trim)
                .filter(|ep| !ep.is_empty())
            {
                return vec![(name.to_string(), dir.join(ep))];
            }

            if let Some(col) = &manifest.collection {
                let mut targets = Vec::new();
                for member in &col.plugins {
                    let linked = plugins_dir.join(member).with_extension("lua");
                    let inside = dir.join(format!("{member}.lua"));
                    let path = if linked.exists() {
                        linked
                    } else if inside.exists() {
                        inside
                    } else {
                        continue;
                    };
                    targets.push((member.clone(), path));
                }
                return targets;
            }
        }
    }

    Vec::new()
}

pub fn build_listing(plugins_dir: &std::path::Path, enabled: &HashSet<String>) -> Listing {
    let mut collections: Vec<CollectionView> = Vec::new();
    let mut standalones: Vec<StandaloneView> = Vec::new();
    let mut member_names: HashSet<String> = HashSet::new();
    let mut dir_plugins: Vec<String> = Vec::new();
    let mut lua_files: Vec<String> = Vec::new();

    let Ok(entries) = fs::read_dir(plugins_dir) else {
        return Listing::default();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if let Ok(manifest) = load_manifest(&path) {
                if let Some(col) = &manifest.collection {
                    let col_enabled = enabled.contains(&name);
                    let members: Vec<MemberView> = col
                        .plugins
                        .iter()
                        .map(|m| {
                            member_names.insert(m.clone());
                            MemberView {
                                name: m.clone(),
                                enabled: col_enabled || enabled.contains(m),
                            }
                        })
                        .collect();
                    let any = members.iter().any(|m| m.enabled);
                    let all = members.iter().all(|m| m.enabled);
                    let status = if all && !members.is_empty() {
                        PluginStatus::Enabled
                    } else if any {
                        PluginStatus::Partial
                    } else {
                        PluginStatus::Disabled
                    };
                    collections.push(CollectionView {
                        name,
                        status,
                        members,
                    });
                    continue;
                }
            }
            dir_plugins.push(name);
        } else if path.extension().and_then(|e| e.to_str()) == Some("lua") {
            if let Some(name) = path
                .file_stem()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            {
                lua_files.push(name);
            }
        }
    }

    for name in dir_plugins.into_iter().chain(lua_files) {
        if member_names.contains(&name) {
            continue;
        }
        let enabled = enabled.contains(&name);
        standalones.push(StandaloneView { name, enabled });
    }

    collections.sort_by(|a, b| a.name.cmp(&b.name));
    standalones.sort_by(|a, b| a.name.cmp(&b.name));
    Listing {
        collections,
        standalones,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn collection_manifest(name: &str, members: &[&str]) -> String {
        let list = members
            .iter()
            .map(|m| format!("\"{m}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("name = \"{name}\"\nversion = \"0.1.0\"\n\n[collection]\nplugins = [{list}]\n")
    }

    #[test]
    fn resolve_standalone_lua_file() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        write(&plugins.join("stay-put.lua"), "-- stay");

        let targets = resolve_load_targets(plugins, "stay-put");

        assert_eq!(
            targets,
            vec![("stay-put".to_string(), plugins.join("stay-put.lua"))]
        );
    }

    #[test]
    fn resolve_collection_loads_all_members_from_dir() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        let col = plugins.join("personal");
        write(
            &col.join("mplug.toml"),
            &collection_manifest("personal", &["a", "b"]),
        );
        write(&col.join("a.lua"), "-- a");
        write(&col.join("b.lua"), "-- b");

        let targets = resolve_load_targets(plugins, "personal");

        assert_eq!(
            targets,
            vec![
                ("a".to_string(), col.join("a.lua")),
                ("b".to_string(), col.join("b.lua")),
            ]
        );
    }

    #[test]
    fn resolve_collection_skips_missing_member_file() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        let col = plugins.join("personal");
        write(
            &col.join("mplug.toml"),
            &collection_manifest("personal", &["a", "missing"]),
        );
        write(&col.join("a.lua"), "-- a");

        let targets = resolve_load_targets(plugins, "personal");

        assert_eq!(targets, vec![("a".to_string(), col.join("a.lua"))]);
    }

    #[test]
    fn resolve_dir_with_entry_point() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        let p = plugins.join("solo");
        write(
            &p.join("mplug.toml"),
            "name = \"solo\"\nversion = \"0.1.0\"\nentry_point = \"init.lua\"\n",
        );
        write(&p.join("init.lua"), "-- init");

        let targets = resolve_load_targets(plugins, "solo");

        assert_eq!(targets, vec![("solo".to_string(), p.join("init.lua"))]);
    }

    #[test]
    fn resolve_unknown_returns_empty() {
        let dir = tempdir().unwrap();
        assert!(resolve_load_targets(dir.path(), "nope").is_empty());
    }

    #[test]
    fn build_listing_nests_collection_members_when_collection_enabled() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        let col = plugins.join("personal");
        write(
            &col.join("mplug.toml"),
            &collection_manifest("personal", &["a", "b"]),
        );
        write(&col.join("a.lua"), "-- a");
        write(&col.join("b.lua"), "-- b");
        write(&plugins.join("stay-put.lua"), "-- s");

        let mut enabled = HashSet::new();
        enabled.insert("personal".to_string());
        let listing = build_listing(plugins, &enabled);

        assert_eq!(listing.collections.len(), 1);
        let c = &listing.collections[0];
        assert_eq!(c.name, "personal");
        assert_eq!(c.status, PluginStatus::Enabled);
        assert_eq!(
            c.members,
            vec![
                MemberView {
                    name: "a".into(),
                    enabled: true
                },
                MemberView {
                    name: "b".into(),
                    enabled: true
                },
            ]
        );
        assert_eq!(
            listing.standalones,
            vec![StandaloneView {
                name: "stay-put".into(),
                enabled: false
            }]
        );
    }

    #[test]
    fn build_listing_collection_partial_when_member_enabled_individually() {
        let dir = tempdir().unwrap();
        let plugins = dir.path();
        let col = plugins.join("useful");
        write(
            &col.join("mplug.toml"),
            &collection_manifest("useful", &["x", "y"]),
        );
        write(&col.join("x.lua"), "-- x");
        write(&col.join("y.lua"), "-- y");
        std::os::unix::fs::symlink(col.join("x.lua"), plugins.join("x.lua")).unwrap();

        let mut enabled = HashSet::new();
        enabled.insert("x".to_string());
        let listing = build_listing(plugins, &enabled);

        assert_eq!(listing.collections.len(), 1);
        let c = &listing.collections[0];
        assert_eq!(c.status, PluginStatus::Partial);
        assert_eq!(
            c.members,
            vec![
                MemberView {
                    name: "x".into(),
                    enabled: true
                },
                MemberView {
                    name: "y".into(),
                    enabled: false
                },
            ]
        );
        assert!(
            listing.standalones.is_empty(),
            "members should not be listed at top level"
        );
    }

    #[test]
    fn still_enabled_members_keeps_individually_enabled_entries() {
        let members = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut enabled = HashSet::new();
        enabled.insert("b".to_string());

        assert_eq!(still_enabled_members(&members, &enabled), vec!["b"]);

        let none: HashSet<String> = HashSet::new();
        assert!(still_enabled_members(&members, &none).is_empty());
    }
}

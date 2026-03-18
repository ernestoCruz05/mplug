use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct CollectionSection {
    pub plugins: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub entry_point: Option<String>,
    pub collection: Option<CollectionSection>,
}

pub fn load_manifest(plugin_dir: &Path) -> Result<PluginManifest> {
    let manifest_path = plugin_dir.join("mplug.toml");
    let content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Cannot read manifest at {}", manifest_path.display()))?;
    let manifest: PluginManifest =
        toml::from_str(&content).context("Manifest is not valid TOML")?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &PluginManifest) -> Result<()> {
    if manifest.name.trim().is_empty() {
        bail!("manifest is missing required field: name");
    }
    if manifest.version.trim().is_empty() {
        bail!("manifest is missing required field: version");
    }
    match &manifest.collection {
        Some(col) if !col.plugins.is_empty() => {
            for plugin in &col.plugins {
                if plugin.trim().is_empty() {
                    bail!("collection.plugins contains an empty name");
                }
            }
        }
        _ => {
            if manifest.entry_point.as_deref().unwrap_or("").trim().is_empty() {
                bail!("manifest must have entry_point or a non-empty [collection] plugins list");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> PluginManifest {
        PluginManifest {
            name: "my-plugin".to_string(),
            version: "0.1.0".to_string(),
            entry_point: Some("init.lua".to_string()),
            collection: None,
        }
    }

    fn valid_collection_manifest() -> PluginManifest {
        PluginManifest {
            name: "my-bundle".to_string(),
            version: "1.0.0".to_string(),
            entry_point: None,
            collection: Some(CollectionSection {
                plugins: vec!["autotile".to_string(), "focus-history".to_string()],
            }),
        }
    }

    #[test]
    fn test_valid_manifest_passes_validation() {
        assert!(validate_manifest(&valid_manifest()).is_ok());
    }

    #[test]
    fn test_valid_collection_manifest_passes_validation() {
        assert!(validate_manifest(&valid_collection_manifest()).is_ok());
    }

    #[test]
    fn test_empty_name_returns_err_with_name() {
        let manifest = PluginManifest {
            name: "".to_string(),
            ..valid_manifest()
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(
            err.to_string().contains("name"),
            "Error should mention 'name': {}",
            err
        );
    }

    #[test]
    fn test_empty_version_returns_err_with_version() {
        let manifest = PluginManifest {
            version: "".to_string(),
            ..valid_manifest()
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(
            err.to_string().contains("version"),
            "Error should mention 'version': {}",
            err
        );
    }

    #[test]
    fn test_no_entry_point_no_collection_fails() {
        let manifest = PluginManifest {
            name: "my-plugin".to_string(),
            version: "0.1.0".to_string(),
            entry_point: None,
            collection: None,
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn test_empty_entry_point_no_collection_fails() {
        let manifest = PluginManifest {
            name: "my-plugin".to_string(),
            version: "0.1.0".to_string(),
            entry_point: Some("".to_string()),
            collection: None,
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn test_collection_with_empty_plugin_name_fails() {
        let manifest = PluginManifest {
            collection: Some(CollectionSection {
                plugins: vec!["".to_string()],
            }),
            entry_point: None,
            ..valid_collection_manifest()
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn test_toml_parse_valid() {
        let toml_str = r#"
name = "my-plugin"
version = "0.1.0"
entry_point = "init.lua"
"#;
        let m: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.name, "my-plugin");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.entry_point.as_deref(), Some("init.lua"));
        assert!(m.collection.is_none());
    }

    #[test]
    fn test_toml_parse_collection() {
        let toml_str = r#"
name = "my-bundle"
version = "1.0.0"

[collection]
plugins = ["carousel", "all-float", "autotile"]
"#;
        let m: PluginManifest = toml::from_str(toml_str).unwrap();
        let col = m.collection.unwrap();
        assert_eq!(col.plugins, vec!["carousel", "all-float", "autotile"]);
        assert!(validate_manifest(&PluginManifest {
            name: m.name,
            version: m.version,
            entry_point: m.entry_point,
            collection: Some(col),
        })
        .is_ok());
    }

    #[test]
    fn test_toml_parse_missing_entry_point_fails_validation() {
        let toml_str = r#"
name = "my-plugin"
version = "0.1.0"
"#;
        let m: PluginManifest = toml::from_str(toml_str).unwrap();
        assert!(validate_manifest(&m).is_err());
    }
}

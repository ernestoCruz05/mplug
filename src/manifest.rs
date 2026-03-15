use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub entry_point: String,
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
    if manifest.entry_point.trim().is_empty() {
        bail!("manifest is missing required field: entry_point");
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
            entry_point: "init.lua".to_string(),
        }
    }

    #[test]
    fn test_valid_manifest_passes_validation() {
        let manifest = valid_manifest();
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn test_empty_name_returns_err_with_name() {
        let manifest = PluginManifest {
            name: "".to_string(),
            version: "0.1.0".to_string(),
            entry_point: "init.lua".to_string(),
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
            name: "my-plugin".to_string(),
            version: "".to_string(),
            entry_point: "init.lua".to_string(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(
            err.to_string().contains("version"),
            "Error should mention 'version': {}",
            err
        );
    }

    #[test]
    fn test_empty_entry_point_returns_err_with_entry_point() {
        let manifest = PluginManifest {
            name: "my-plugin".to_string(),
            version: "0.1.0".to_string(),
            entry_point: "".to_string(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(
            err.to_string().contains("entry_point"),
            "Error should mention 'entry_point': {}",
            err
        );
    }

    #[test]
    fn test_toml_parse_valid() {
        let toml_str = r#"
name = "my-plugin"
version = "0.1.0"
entry_point = "init.lua"
"#;
        let manifest: Result<PluginManifest, _> = toml::from_str(toml_str);
        assert!(manifest.is_ok());
        let m = manifest.unwrap();
        assert_eq!(m.name, "my-plugin");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.entry_point, "init.lua");
    }

    #[test]
    fn test_toml_parse_missing_required_field_fails() {
        let toml_str = r#"
name = "my-plugin"
version = "0.1.0"
"#;
        let manifest: Result<PluginManifest, _> = toml::from_str(toml_str);
        assert!(
            manifest.is_err(),
            "Parsing should fail when entry_point is missing"
        );
    }
}

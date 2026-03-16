use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RepositoryConfig {
    #[serde(default)]
    pub serve: ServeSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeSettings {
    #[serde(default = "default_port")]
    pub port: u16,
}

impl RepositoryConfig {
    pub fn load(repository_root: &Utf8Path) -> Result<Self> {
        let config_path = repository_root.join(".repo").join("config.toml");
        let config_text = std::fs::read_to_string(config_path.as_std_path())
            .context("failed to read .repo/config.toml")?;
        let config: Self =
            toml::from_str(&config_text).context("failed to parse .repo/config.toml")?;
        Ok(config)
    }
}

impl Default for ServeSettings {
    fn default() -> Self {
        Self {
            port: default_port(),
        }
    }
}

fn default_port() -> u16 {
    3000
}

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RepositoryConfig {
    #[serde(default)]
    pub serve: ServeSettings,
    #[serde(default)]
    pub policy: Vec<PolicyRule>,
    #[serde(default)]
    pub mount: Vec<MountRule>,
    #[serde(default)]
    pub plugin: Vec<PluginConfig>,
    #[serde(default)]
    pub task: Vec<TaskConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeSettings {
    #[serde(default = "default_port")]
    pub port: u16,
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

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyRule {
    pub path: String,
    #[serde(rename = "GET")]
    pub get: bool,
    #[serde(rename = "POST")]
    pub post: bool,
    #[serde(rename = "PUT")]
    pub put: bool,
    #[serde(rename = "DELETE")]
    pub delete: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MountRule {
    pub url_prefix: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub runner: String,
    pub command: Vec<String>,
    pub trigger: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfig {
    pub name: String,
    pub steps: Vec<String>,
}

impl RepositoryConfig {
    pub fn load(repository_root: &Utf8Path) -> Result<Self> {
        let config_path = repository_root.join(".repo").join("config.toml");
        let config_text = std::fs::read_to_string(config_path.as_std_path())
            .context("failed to read .repo/config.toml")?;
        let config: Self =
            toml::from_str(&config_text).context("failed to parse .repo/config.toml")?;
        config.validate(repository_root)?;
        Ok(config)
    }

    fn validate(&self, repository_root: &Utf8Path) -> Result<()> {
        for policy in &self.policy {
            if policy.path.starts_with(".repo/") || policy.path == ".repo/" || policy.path == ".repo" {
                bail!("policy path must not target .repo/");
            }
        }

        for mount in &self.mount {
            if !mount.url_prefix.starts_with('/') || !mount.url_prefix.ends_with('/') {
                bail!("mount url_prefix must start and end with /");
            }
            if !mount.source.starts_with(".repo/generated/") {
                bail!("mount source must be under .repo/generated/");
            }

            let relative = Utf8Path::new(mount.url_prefix.trim_matches('/'));
            if !relative.as_str().is_empty() {
                let target = repository_root.join(relative);
                if target.is_dir() {
                    bail!("mount url_prefix conflicts with repository directory: {}", mount.url_prefix);
                }
            }
        }

        for plugin in &self.plugin {
            if plugin.name.is_empty() {
                bail!("plugin name must not be empty");
            }
            if plugin.runner != "command" {
                bail!("unsupported plugin runner: {}", plugin.runner);
            }
            if plugin.command.is_empty() {
                bail!("plugin command must not be empty");
            }
        }

        Ok(())
    }

    pub fn find_plugin(&self, name: &str) -> Option<&PluginConfig> {
        self.plugin.iter().find(|plugin| plugin.name == name)
    }

    pub fn find_task(&self, name: &str) -> Option<&TaskConfig> {
        self.task.iter().find(|task| task.name == name)
    }
}

impl MountRule {
    pub fn source_relative_path(&self) -> Utf8PathBuf {
        Utf8PathBuf::from(self.source.trim_end_matches('/'))
    }
}

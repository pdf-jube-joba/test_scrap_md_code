use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use glob::Pattern;
use tokio::process::Command;

use crate::config::{PluginConfig, RepositoryConfig, TaskConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTrigger {
    Get,
    Post,
    Put,
    Delete,
    Manual,
}

impl PluginTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub repository_root: Utf8PathBuf,
    pub repository_name: String,
    pub plugin_name: String,
    pub output_directory: Utf8PathBuf,
    pub cache_directory: Utf8PathBuf,
    pub path: Option<String>,
    pub user_identity: String,
}

pub struct PluginRunner<'a> {
    repository_root: &'a Utf8Path,
    repository_name: &'a str,
    config: &'a RepositoryConfig,
}

impl<'a> PluginRunner<'a> {
    pub fn new(
        repository_root: &'a Utf8Path,
        repository_name: &'a str,
        config: &'a RepositoryConfig,
    ) -> Self {
        Self {
            repository_root,
            repository_name,
            config,
        }
    }

    pub async fn run_task(&self, task_name: &str) -> Result<()> {
        let task = self
            .config
            .find_task(task_name)
            .with_context(|| format!("task not found: {task_name}"))?;

        for step in &task.steps {
            let plugin = self
                .config
                .find_plugin(step)
                .with_context(|| format!("plugin not found for task step: {step}"))?;
            self.run_plugin(plugin, PluginTrigger::Manual, None, "").await?;
        }

        Ok(())
    }

    pub async fn run_manual_plugin(&self, plugin_name: &str, user_identity: &str) -> Result<()> {
        let plugin = self
            .config
            .find_plugin(plugin_name)
            .with_context(|| format!("plugin not found: {plugin_name}"))?;

        if plugin.trigger != "manual" {
            bail!("plugin is not manual: {plugin_name}");
        }

        self.run_plugin(plugin, PluginTrigger::Manual, None, user_identity)
            .await
    }

    pub async fn run_hook_if_matched(
        &self,
        trigger: PluginTrigger,
        path: &str,
        user_identity: &str,
    ) -> Result<()> {
        for plugin in &self.config.plugin {
            if parse_trigger(&plugin.trigger)? != trigger {
                continue;
            }

            let Some(pattern) = &plugin.path else {
                continue;
            };
            if !Pattern::new(pattern)
                .with_context(|| format!("invalid plugin path pattern: {pattern}"))?
                .matches(path)
            {
                continue;
            }

            self.run_plugin(plugin, trigger, Some(path), user_identity).await?;
        }

        Ok(())
    }

    async fn run_plugin(
        &self,
        plugin: &PluginConfig,
        trigger: PluginTrigger,
        path: Option<&str>,
        user_identity: &str,
    ) -> Result<()> {
        let context = PluginContext {
            repository_root: self.repository_root.to_owned(),
            repository_name: self.repository_name.to_owned(),
            plugin_name: plugin.name.clone(),
            output_directory: self
                .repository_root
                .join(".repo")
                .join("generated")
                .join(&plugin.name),
            cache_directory: self
                .repository_root
                .join(".repo")
                .join("cache")
                .join(&plugin.name),
            path: path.map(ToOwned::to_owned),
            user_identity: user_identity.to_owned(),
        };

        tokio::fs::create_dir_all(context.output_directory.as_std_path())
            .await
            .context("failed to create plugin output directory")?;
        tokio::fs::create_dir_all(context.cache_directory.as_std_path())
            .await
            .context("failed to create plugin cache directory")?;

        let program = expand_placeholder(&plugin.command[0], &context)?;
        let args = plugin.command[1..]
            .iter()
            .map(|arg| expand_placeholder(arg, &context))
            .collect::<Result<Vec<_>>>()?;

        tracing::info!(
            plugin = %plugin.name,
            trigger = %trigger.as_str(),
            path = %path.unwrap_or(""),
            "running plugin"
        );

        let output = Command::new(&program)
            .args(&args)
            .current_dir(context.repository_root.as_std_path())
            .env("WORKSPACE_FS_REPOSITORY_ROOT", context.repository_root.as_str())
            .env("WORKSPACE_FS_REPOSITORY_NAME", &context.repository_name)
            .env("WORKSPACE_FS_PLUGIN_NAME", &context.plugin_name)
            .env("WORKSPACE_FS_OUTPUT_DIRECTORY", context.output_directory.as_str())
            .env("WORKSPACE_FS_CACHE_DIRECTORY", context.cache_directory.as_str())
            .env("WORKSPACE_FS_TRIGGER", trigger.as_str())
            .env("WORKSPACE_FS_PATH", context.path.as_deref().unwrap_or(""))
            .env("WORKSPACE_FS_USER_IDENTITY", &context.user_identity)
            .output()
            .await
            .with_context(|| format!("failed to run plugin: {}", plugin.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("plugin failed: {}: {}", plugin.name, stderr.trim());
        }

        Ok(())
    }
}

fn parse_trigger(trigger: &str) -> Result<PluginTrigger> {
    match trigger {
        "GET" => Ok(PluginTrigger::Get),
        "POST" => Ok(PluginTrigger::Post),
        "PUT" => Ok(PluginTrigger::Put),
        "DELETE" => Ok(PluginTrigger::Delete),
        "manual" => Ok(PluginTrigger::Manual),
        _ => bail!("unsupported plugin trigger: {trigger}"),
    }
}

fn expand_placeholder(input: &str, context: &PluginContext) -> Result<String> {
    let mut value = input.to_owned();
    let replacements = [
        ("{REPOSITORY_NAME}", context.repository_name.as_str()),
        ("{PLUGIN_NAME}", context.plugin_name.as_str()),
        ("{OUTPOST_DIRECTORY}", context.output_directory.as_str()),
        ("{OUTPUT_DIRECTORY}", context.output_directory.as_str()),
        ("{GET.PATH}", context.path.as_deref().unwrap_or("")),
        ("{GET.USER-IDENTITY}", context.user_identity.as_str()),
        ("{POST.PATH}", context.path.as_deref().unwrap_or("")),
        ("{POST.USER-IDENTITY}", context.user_identity.as_str()),
        ("{PUT.PATH}", context.path.as_deref().unwrap_or("")),
        ("{PUT.USER-IDENTITY}", context.user_identity.as_str()),
        ("{DELETE.PATH}", context.path.as_deref().unwrap_or("")),
        ("{DELETE.USER-IDENTITY}", context.user_identity.as_str()),
    ];

    for (from, to) in replacements {
        value = value.replace(from, to);
    }

    if value.contains('{') || value.contains('}') {
        bail!("unknown placeholder in plugin command: {input}");
    }

    Ok(value)
}

#[allow(dead_code)]
fn _task_reference(_task: &TaskConfig) {}

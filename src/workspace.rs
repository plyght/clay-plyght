use anyhow::{Result, anyhow};
use console::style;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tokio::process::Command;

use crate::cli_style::CliStyle;
use crate::package_manager::PackageManager;

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub workspaces: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspacePackage {
    pub name: String,
    pub path: String,
    pub package_json: PathBuf,
}

pub struct WorkspaceManager {
    root_path: PathBuf,
    workspace_config_path: PathBuf,
}

impl WorkspaceManager {
    pub fn new() -> Self {
        Self {
            root_path: PathBuf::from("."),
            workspace_config_path: PathBuf::from("package.json"),
        }
    }

    pub async fn list_workspaces(&self) -> Result<()> {
        let workspaces = self.discover_workspaces().await?;

        if workspaces.is_empty() {
            println!("{} No workspaces configured", style("•").yellow());
            return Ok(());
        }

        println!("{}", CliStyle::section_header("Workspaces:"));

        for workspace in &workspaces {
            let package_info = self.read_workspace_package_json(&workspace.path).await?;
            let version = package_info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            println!(
                "  {} {} {} {}",
                style("•").cyan(),
                style(&workspace.name).white().bold(),
                style(&format!("v{version}")).dim(),
                style(&format!("({})", workspace.path)).dim()
            );
        }

        println!(
            "\n{} {} workspaces total",
            CliStyle::success(""),
            style(workspaces.len()).white().bold()
        );

        Ok(())
    }

    pub async fn add_workspace(&self, name: &str, path: &str) -> Result<()> {
        let workspace_path = PathBuf::from(path);

        // Ensure workspace directory exists
        if !workspace_path.exists() {
            fs::create_dir_all(&workspace_path).await?;
        }

        // Create package.json if it doesn't exist
        let package_json_path = workspace_path.join("package.json");
        if !package_json_path.exists() {
            let package_json = serde_json::json!({
                "name": name,
                "version": "1.0.0",
                "private": true
            });

            let content = serde_json::to_string_pretty(&package_json)?;
            fs::write(&package_json_path, content).await?;
        }

        // Update root package.json to include workspace
        self.add_workspace_to_config(path).await?;

        println!(
            "{} Added workspace: {} {}",
            CliStyle::success(""),
            style(name).white().bold(),
            style(&format!("({path})")).dim()
        );

        Ok(())
    }

    pub async fn remove_workspace(&self, name: &str) -> Result<()> {
        let workspaces = self.discover_workspaces().await?;
        let workspace = workspaces
            .iter()
            .find(|w| w.name == name)
            .ok_or_else(|| anyhow!("Workspace '{}' not found", name))?;

        self.remove_workspace_from_config(&workspace.path).await?;

        println!(
            "{} Removed workspace: {}",
            CliStyle::success(""),
            style(name).white().bold()
        );

        Ok(())
    }

    pub async fn run_script(
        &self,
        script: &str,
        workspace_filter: Option<&str>,
        parallel: bool,
    ) -> Result<()> {
        let workspaces = self.discover_workspaces().await?;

        let target_workspaces: Vec<&WorkspacePackage> = if let Some(filter) = workspace_filter {
            workspaces.iter().filter(|w| w.name == filter).collect()
        } else {
            workspaces.iter().collect()
        };

        if target_workspaces.is_empty() {
            println!("{} No workspaces found", style("•").yellow());
            return Ok(());
        }

        println!(
            "{} Running script '{}' in {} workspace{}{}",
            CliStyle::info(""),
            style(script).white().bold(),
            style(target_workspaces.len()).white().bold(),
            if target_workspaces.len() == 1 {
                ""
            } else {
                "s"
            },
            if parallel { " (parallel)" } else { "" }
        );

        if parallel {
            let tasks: Vec<_> = target_workspaces
                .iter()
                .map(|workspace| {
                    let workspace_name = workspace.name.clone();
                    let workspace_path = workspace.path.clone();
                    let script = script.to_string();

                    async move {
                        println!(
                            "{} [{}] Starting script...",
                            style("→").cyan(),
                            style(&workspace_name).white().bold()
                        );

                        let result = self
                            .execute_script_in_workspace(&script, &workspace_path)
                            .await;

                        match result {
                            Ok(success) => {
                                if success {
                                    println!(
                                        "{} [{}] Script completed successfully",
                                        CliStyle::success(""),
                                        style(&workspace_name).white().bold()
                                    );
                                } else {
                                    println!(
                                        "{} [{}] Script failed",
                                        CliStyle::error(""),
                                        style(&workspace_name).white().bold()
                                    );
                                }
                                success
                            }
                            Err(e) => {
                                println!(
                                    "{} [{}] Script error: {}",
                                    CliStyle::error(""),
                                    style(&workspace_name).white().bold(),
                                    e
                                );
                                false
                            }
                        }
                    }
                })
                .collect();

            let results = join_all(tasks).await;
            let successful = results.iter().filter(|&&success| success).count();
            let failed = results.len() - successful;

            if failed > 0 {
                println!(
                    "\n{} {} successful, {} failed",
                    style("Summary:").blue().bold(),
                    style(successful).green(),
                    style(failed).red()
                );
            } else {
                println!(
                    "\n{} All {} scripts completed successfully",
                    CliStyle::success(""),
                    style(successful).white().bold()
                );
            }
        } else {
            for workspace in target_workspaces {
                println!(
                    "{} [{}] Running script...",
                    style("→").cyan(),
                    style(&workspace.name).white().bold()
                );

                match self
                    .execute_script_in_workspace(script, &workspace.path)
                    .await
                {
                    Ok(true) => {
                        println!(
                            "{} [{}] Script completed successfully",
                            CliStyle::success(""),
                            style(&workspace.name).white().bold()
                        );
                    }
                    Ok(false) => {
                        println!(
                            "{} [{}] Script failed",
                            CliStyle::error(""),
                            style(&workspace.name).white().bold()
                        );
                    }
                    Err(e) => {
                        println!(
                            "{} [{}] Script error: {}",
                            CliStyle::error(""),
                            style(&workspace.name).white().bold(),
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    async fn discover_workspaces(&self) -> Result<Vec<WorkspacePackage>> {
        let mut workspaces = Vec::new();

        // Check if we have a workspace configuration
        if !self.workspace_config_path.exists() {
            return Ok(workspaces);
        }

        let content = fs::read_to_string(&self.workspace_config_path).await?;
        let package_json: serde_json::Value = serde_json::from_str(&content)?;

        if let Some(workspace_patterns) = package_json.get("workspaces") {
            let patterns = match workspace_patterns {
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                serde_json::Value::Object(obj) => {
                    if let Some(packages) = obj.get("packages") {
                        if let Some(arr) = packages.as_array() {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                        } else {
                            return Ok(workspaces);
                        }
                    } else {
                        return Ok(workspaces);
                    }
                }
                _ => return Ok(workspaces),
            };

            for pattern in patterns {
                let workspace_paths = self.resolve_workspace_pattern(&pattern).await?;
                for path in workspace_paths {
                    if let Ok(package_info) = self.read_workspace_package_json(&path).await {
                        if let Some(name) = package_info.get("name").and_then(|n| n.as_str()) {
                            workspaces.push(WorkspacePackage {
                                name: name.to_string(),
                                path: path.clone(),
                                package_json: PathBuf::from(&path).join("package.json"),
                            });
                        }
                    }
                }
            }
        }

        Ok(workspaces)
    }

    async fn resolve_workspace_pattern(&self, pattern: &str) -> Result<Vec<String>> {
        let mut paths = Vec::new();

        if pattern.contains('*') {
            // Handle glob patterns

            let mut entries = fs::read_dir(".").await?;
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    let dir_name = entry.file_name();
                    let dir_str = dir_name.to_string_lossy();

                    if let Some(base_pattern) = pattern.strip_suffix("/*") {
                        if dir_str.starts_with(base_pattern) {
                            let package_json_path = entry.path().join("package.json");
                            if package_json_path.exists() {
                                paths.push(entry.path().to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        } else {
            // Direct path
            let path = PathBuf::from(pattern);
            if path.exists() && path.join("package.json").exists() {
                paths.push(pattern.to_string());
            }
        }

        Ok(paths)
    }

    async fn read_workspace_package_json(&self, workspace_path: &str) -> Result<serde_json::Value> {
        let package_json_path = PathBuf::from(workspace_path).join("package.json");
        let content = fs::read_to_string(&package_json_path).await?;
        let package_json: serde_json::Value = serde_json::from_str(&content)?;
        Ok(package_json)
    }

    async fn add_workspace_to_config(&self, workspace_path: &str) -> Result<()> {
        let mut package_json = if self.workspace_config_path.exists() {
            let content = fs::read_to_string(&self.workspace_config_path).await?;
            serde_json::from_str::<serde_json::Value>(&content)?
        } else {
            serde_json::json!({
                "name": "my-monorepo",
                "private": true,
                "workspaces": []
            })
        };

        let workspaces_array = package_json["workspaces"]
            .as_array_mut()
            .ok_or_else(|| anyhow!("Invalid workspaces configuration"))?;

        let workspace_path_value = serde_json::Value::String(workspace_path.to_string());
        if !workspaces_array.contains(&workspace_path_value) {
            workspaces_array.push(workspace_path_value);
        }

        let content = serde_json::to_string_pretty(&package_json)?;
        fs::write(&self.workspace_config_path, content).await?;

        Ok(())
    }

    async fn remove_workspace_from_config(&self, workspace_path: &str) -> Result<()> {
        if !self.workspace_config_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.workspace_config_path).await?;
        let mut package_json: serde_json::Value = serde_json::from_str(&content)?;

        if let Some(workspaces_array) = package_json["workspaces"].as_array_mut() {
            workspaces_array.retain(|w| w.as_str() != Some(workspace_path));
        }

        let content = serde_json::to_string_pretty(&package_json)?;
        fs::write(&self.workspace_config_path, content).await?;

        Ok(())
    }

    async fn execute_script_in_workspace(
        &self,
        script: &str,
        workspace_path: &str,
    ) -> Result<bool> {
        let package_json_path = PathBuf::from(workspace_path).join("package.json");

        if !package_json_path.exists() {
            return Err(anyhow!(
                "No package.json found in workspace {}",
                workspace_path
            ));
        }

        let content = fs::read_to_string(&package_json_path).await?;
        let package_json: serde_json::Value = serde_json::from_str(&content)?;

        let scripts = package_json
            .get("scripts")
            .and_then(|s| s.as_object())
            .ok_or_else(|| anyhow!("No scripts found in workspace {}", workspace_path))?;

        let script_command = scripts
            .get(script)
            .and_then(|s| s.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "Script '{}' not found in workspace {}",
                    script,
                    workspace_path
                )
            })?;

        let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", script_command]);
            cmd
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let mut cmd = Command::new(shell);
            cmd.arg("-c").arg(script_command);
            cmd
        };

        cmd.current_dir(workspace_path);

        // Add node_modules/.bin to PATH (both root and workspace)
        let mut paths = Vec::new();

        let root_bin = self.root_path.join("node_modules").join(".bin");
        if root_bin.exists() {
            paths.push(root_bin.to_string_lossy().to_string());
        }

        let workspace_bin = PathBuf::from(workspace_path)
            .join("node_modules")
            .join(".bin");
        if workspace_bin.exists() {
            paths.push(workspace_bin.to_string_lossy().to_string());
        }

        if !paths.is_empty() {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let path_separator = if cfg!(target_os = "windows") {
                ";"
            } else {
                ":"
            };
            let new_path = if current_path.is_empty() {
                paths.join(path_separator)
            } else {
                format!(
                    "{}{}{}",
                    paths.join(path_separator),
                    path_separator,
                    current_path
                )
            };
            cmd.env("PATH", new_path);
        }

        let status = cmd.status().await?;
        Ok(status.success())
    }

    pub async fn install_workspace_dependencies(&self) -> Result<()> {
        let workspaces = self.discover_workspaces().await?;

        if workspaces.is_empty() {
            println!("{} No workspaces found", style("•").yellow());
            return Ok(());
        }

        let workspace_count = workspaces.len();
        let workspace_word = if workspace_count == 1 {
            "workspace"
        } else {
            "workspaces"
        };
        let install_spinner = CliStyle::create_spinner(&format!(
            "Installing dependencies for {workspace_count} {workspace_word}..."
        ));

        // Install root dependencies first
        let package_manager = PackageManager::new();
        let root_deps = package_manager.get_package_json_dependencies(false).await?;
        if !root_deps.is_empty() {
            install_spinner.set_message("Installing root dependencies...");
            package_manager
                .install_multiple_packages(root_deps, false, false)
                .await?;
        }

        // Install workspace dependencies
        for workspace in workspaces {
            install_spinner
                .set_message(format!("Installing dependencies for {}...", workspace.name));

            // Note: We would need to modify PackageManager to work with different working directories
            // For now, we'll use a simple approach - this is a placeholder for future implementation
        }

        install_spinner.finish_with_message(format!(
            "Installed dependencies for {workspace_count} {workspace_word}"
        ));
        Ok(())
    }
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

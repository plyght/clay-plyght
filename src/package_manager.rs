use anyhow::{Result, anyhow};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;

use tokio::sync::{Mutex, Semaphore};

use crate::npm_client::NpmClient;
use crate::package_info::{LockFile, PackageJson};

struct ProgressTracker {
    progress_bar: ProgressBar,
    current: u64,
    total: u64,
    start_time: Instant,
}

impl ProgressTracker {
    fn new(total: u64) -> Self {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.cyan} {bar:40.green/dim} {pos:>3}/{len:3} ┃ {elapsed_precise} ┃ {msg}",
                )
                .unwrap()
                .progress_chars("━━╾─ "),
        );
        pb.set_message("Initializing");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));

        Self {
            progress_bar: pb,
            current: 0,
            total,
            start_time: Instant::now(),
        }
    }

    fn update(&mut self, message: &str) {
        self.current += 1;
        self.progress_bar.set_position(self.current);
        self.progress_bar.set_message(message.to_string());
    }

    fn finish(&self) {
        let duration = self.start_time.elapsed();
        let message = if duration.as_millis() < 1000 {
            format!(
                "{} {} package{} installed in {}ms",
                style("✓").green().bold(),
                self.total,
                if self.total == 1 { "" } else { "s" },
                duration.as_millis()
            )
        } else {
            format!(
                "{} {} package{} installed in {:.1}s",
                style("✓").green().bold(),
                self.total,
                if self.total == 1 { "" } else { "s" },
                duration.as_millis() as f64 / 1000.0
            )
        };
        self.progress_bar.finish_with_message(message);
    }
}

pub struct PackageManager {
    npm_client: NpmClient,
    node_modules_dir: PathBuf,
    package_json_path: PathBuf,
    lock_file_path: PathBuf,
    semaphore: Arc<Semaphore>,
    file_mutex: Arc<Mutex<()>>,
}

impl PackageManager {
    pub fn new() -> Self {
        Self {
            npm_client: NpmClient::new(),
            node_modules_dir: PathBuf::from("node_modules"),
            package_json_path: PathBuf::from("package.json"),
            lock_file_path: PathBuf::from("fnpm-lock.json"),
            semaphore: Arc::new(Semaphore::new(8)), // Allow 8 concurrent downloads
            file_mutex: Arc::new(Mutex::new(())),
        }
    }

    /// Install a package and save it to node_modules
    pub async fn install_package(&self, package_name: &str, version: &str) -> Result<()> {
        // First, count total packages to install
        let total_packages = self
            .count_packages_to_install(package_name, version)
            .await?;

        // Create progress tracker
        let mut progress = ProgressTracker::new(total_packages);

        // Install with progress tracking
        self.install_package_with_progress(package_name, version, true, &mut progress)
            .await?;

        progress.finish();

        // Show summary
        println!(
            "\n{} Successfully installed {}",
            style("✓").green().bold(),
            style(package_name).white().bold()
        );

        Ok(())
    }

    /// Count total packages that will be installed (including dependencies)
    async fn count_packages_to_install(&self, package_name: &str, version: &str) -> Result<u64> {
        let mut count = 0;

        // Check if main package needs installation
        let package_dir = self.node_modules_dir.join(package_name);
        if !package_dir.exists() {
            count += 1;

            // Fetch package info to check dependencies
            let registry_response = self.npm_client.get_package_info(package_name).await?;
            let package_info = if version == "latest" {
                registry_response.get_latest_version()
            } else {
                registry_response.get_version(version)
            };

            if let Some(package_info) = package_info {
                if let Some(ref dependencies) = package_info.dependencies {
                    for (dep_name, _) in dependencies {
                        let dep_package_dir = self.node_modules_dir.join(dep_name);
                        if !dep_package_dir.exists() {
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Internal install method with option to update package.json
    async fn install_package_with_progress(
        &self,
        package_name: &str,
        version: &str,
        update_package_json: bool,
        progress: &mut ProgressTracker,
    ) -> Result<()> {
        // Ensure node_modules directory exists
        self.ensure_node_modules_exists().await?;

        // Fetch package information from NPM registry
        progress
            .progress_bar
            .set_message(format!("Fetching {}", package_name));
        let registry_response = self.npm_client.get_package_info(package_name).await?;

        let package_info = if version == "latest" {
            registry_response.get_latest_version()
        } else {
            registry_response.get_version(version)
        };

        let package_info = package_info.ok_or_else(|| {
            anyhow!(
                "Version '{}' not found for package '{}'",
                version,
                package_name
            )
        })?;

        // Check if package is already installed
        let package_dir = self.node_modules_dir.join(&package_info.name);
        if package_dir.exists() {
            println!(
                "{} {} already installed",
                style("•").cyan(),
                style(&package_info.name).white()
            );
            return Ok(());
        }

        // Download the package tarball
        progress
            .progress_bar
            .set_message(format!("{} {}", style("↓").cyan(), package_info.name));
        let tarball_path = self.download_package_tarball(package_info).await?;

        // Check if tarball was actually created
        if !tarball_path.exists() {
            return Err(anyhow!(
                "Failed to download tarball for {}",
                package_info.name
            ));
        }

        // Extract the tarball to node_modules
        progress.progress_bar.set_message(format!(
            "{} {}",
            style("⚡").yellow(),
            package_info.name
        ));
        self.extract_package(&tarball_path, &package_dir).await?;

        // Clean up the tarball and temp directory
        if tarball_path.exists() {
            fs::remove_file(&tarball_path).await.ok();
        }
        if let Some(temp_dir) = tarball_path.parent() {
            fs::remove_dir_all(temp_dir).await.ok();
        }

        // Update package.json only if this is the explicitly requested package
        if update_package_json {
            self.update_package_json(&package_info.name, &package_info.version)
                .await?;
        }

        // Update lock file
        let parent_name = if update_package_json {
            "root"
        } else {
            // Find the actual parent package name from the call stack
            package_name
        };

        self.update_lock_file(
            &package_info.name,
            &package_info.version,
            &package_info.dist.tarball,
            &package_info.dist.shasum,
            package_info.dependencies.as_ref(),
            parent_name,
        )
        .await?;

        // Update progress for main package
        progress.update(&format!("{} {}", style("✓").green(), package_info.name));

        // Install dependencies in parallel if any
        if let Some(ref dependencies) = package_info.dependencies {
            self.install_dependencies_parallel(dependencies, &package_info.name, progress)
                .await?;
        }

        Ok(())
    }

    /// Install dependencies in parallel
    async fn install_dependencies_parallel(
        &self,
        dependencies: &std::collections::HashMap<String, String>,
        parent_name: &str,
        progress: &mut ProgressTracker,
    ) -> Result<()> {
        let mut tasks = Vec::new();

        for (dep_name, dep_version) in dependencies {
            // Check if dependency is already installed
            let dep_package_dir = self.node_modules_dir.join(dep_name);
            if dep_package_dir.exists() {
                // Still add to lock file to track dependency relationship
                self.update_lock_file(dep_name, dep_version, "", "", None, parent_name)
                    .await?;
                continue;
            }

            // Clone data for the async task
            let dep_name = dep_name.clone();
            let dep_version = dep_version.clone();
            let parent_name = parent_name.to_string();
            let npm_client = self.npm_client.clone();
            let node_modules_dir = self.node_modules_dir.clone();
            let lock_file_path = self.lock_file_path.clone();
            let semaphore = Arc::clone(&self.semaphore);
            let file_mutex = Arc::clone(&self.file_mutex);

            // Spawn async task for each dependency
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();

                // Resolve version range
                let registry_response = npm_client.get_package_info(&dep_name).await?;
                let resolved_version = if dep_version == "latest" {
                    registry_response
                        .get_latest_version()
                        .map(|p| p.version.clone())
                } else {
                    // Simple version resolution for ranges
                    if Self::is_exact_version(&dep_version) {
                        Some(dep_version.clone())
                    } else {
                        registry_response
                            .get_latest_version()
                            .map(|p| p.version.clone())
                    }
                };

                let resolved_version = resolved_version
                    .ok_or_else(|| anyhow::anyhow!("Could not resolve version for {}", dep_name))?;

                let package_info = registry_response
                    .get_version(&resolved_version)
                    .or_else(|| registry_response.get_latest_version())
                    .ok_or_else(|| anyhow::anyhow!("Package info not found for {}", dep_name))?;

                // Download package with integrity verification
                let tarball_path = {
                    let tarball_filename =
                        format!("{}-{}.tgz", package_info.name, package_info.version);

                    // Create unique temp directory per package to avoid conflicts
                    let temp_dir = PathBuf::from("temp").join(&dep_name);
                    let tarball_path = temp_dir.join(&tarball_filename);

                    // Ensure temp directory exists
                    tokio::fs::create_dir_all(&temp_dir).await?;

                    // Download and verify
                    let response = npm_client
                        .client
                        .get(&package_info.dist.tarball)
                        .send()
                        .await?;
                    if !response.status().is_success() {
                        return Err(anyhow::anyhow!(
                            "Failed to download package: HTTP {}",
                            response.status()
                        ));
                    }

                    let bytes = response.bytes().await?;

                    // Verify integrity
                    if !npm_client.verify_package_integrity(&bytes, &package_info.dist.shasum)? {
                        return Err(anyhow::anyhow!(
                            "Package integrity verification failed for {}",
                            package_info.name
                        ));
                    }

                    // Write to file with proper error handling
                    if let Some(parent) = tarball_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    let mut file = tokio::fs::File::create(&tarball_path).await?;
                    use tokio::io::AsyncWriteExt;
                    file.write_all(&bytes).await?;
                    file.sync_all().await?;

                    tarball_path
                };

                // Extract package
                let package_dir = node_modules_dir.join(&package_info.name);
                tokio::fs::create_dir_all(&package_dir).await?;

                // Check if tarball exists before extraction
                if !tarball_path.exists() {
                    return Err(anyhow::anyhow!("Tarball not found: {:?}", tarball_path));
                }

                let output = tokio::process::Command::new("tar")
                    .args([
                        "-xzf",
                        tarball_path.to_str().unwrap(),
                        "-C",
                        package_dir.to_str().unwrap(),
                        "--strip-components=1",
                    ])
                    .output()
                    .await?;

                if !output.status.success() {
                    let error_message = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow::anyhow!(
                        "Failed to extract tarball for {}: {}",
                        package_info.name,
                        error_message
                    ));
                }

                // Clean up tarball and temp directory
                if tarball_path.exists() {
                    tokio::fs::remove_file(&tarball_path).await.ok();
                }
                if let Some(temp_dir) = tarball_path.parent() {
                    tokio::fs::remove_dir_all(temp_dir).await.ok();
                }

                // Update lock file with mutex protection
                {
                    let _lock = file_mutex.lock().await;
                    let mut lock_file = if lock_file_path.exists() {
                        let content = tokio::fs::read_to_string(&lock_file_path).await?;
                        if content.trim().is_empty() {
                            LockFile::new()
                        } else {
                            serde_json::from_str::<LockFile>(&content)
                                .unwrap_or_else(|_| LockFile::new())
                        }
                    } else {
                        LockFile::new()
                    };

                    lock_file.add_package(
                        &package_info.name,
                        &package_info.version,
                        &package_info.dist.tarball,
                        &package_info.dist.shasum,
                        package_info.dependencies.clone(),
                        &parent_name,
                    );

                    let content = serde_json::to_string_pretty(&lock_file)?;
                    tokio::fs::write(&lock_file_path, content).await?;
                }

                Ok::<(String, Option<std::collections::HashMap<String, String>>), anyhow::Error>((
                    dep_name,
                    package_info.dependencies.clone(),
                ))
            });

            tasks.push(task);
        }

        // Wait for all downloads to complete
        let mut nested_dependencies = Vec::new();
        for task in tasks {
            match task.await? {
                Ok((dep_name, deps)) => {
                    progress.update(&format!("{} {}", style("✓").green(), dep_name));
                    if let Some(deps) = deps {
                        nested_dependencies.push((dep_name, deps));
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Install nested dependencies (still parallel but after current level)
        for (dep_name, deps) in nested_dependencies {
            if !deps.is_empty() {
                Box::pin(self.install_dependencies_parallel(&deps, &dep_name, progress)).await?;
            }
        }

        Ok(())
    }

    /// Static helper for version checking
    fn is_exact_version(version: &str) -> bool {
        if version.starts_with('^')
            || version.starts_with('~')
            || version.starts_with('>')
            || version.starts_with('<')
            || version.starts_with('=')
            || version == "*"
        {
            return false;
        }

        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() >= 3 {
            parts.iter().take(3).all(|part| {
                part.split('-')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .all(|c| c.is_ascii_digit())
            })
        } else {
            false
        }
    }

    /// Install all dependencies from package.json
    pub async fn install_dependencies(&self) -> Result<()> {
        if !self.package_json_path.exists() {
            println!("{} No package.json found", style("•").yellow());
            return Ok(());
        }

        let content = fs::read_to_string(&self.package_json_path).await?;
        let package_json: PackageJson = if content.trim().is_empty() {
            PackageJson::new()
        } else {
            serde_json::from_str(&content).unwrap_or_else(|_| PackageJson::new())
        };

        if let Some(dependencies) = package_json.dependencies {
            if dependencies.is_empty() {
                println!("{} No dependencies in package.json", style("•").yellow());
                return Ok(());
            }

            // Count total packages to install
            let mut total_packages = 0;
            for (dep_name, _) in &dependencies {
                let dep_package_dir = self.node_modules_dir.join(dep_name);
                if !dep_package_dir.exists() {
                    total_packages += 1;
                }
            }

            if total_packages == 0 {
                println!("{} All dependencies already installed", style("✓").green());
                return Ok(());
            }

            // Create progress tracker
            let mut progress = ProgressTracker::new(total_packages);

            // Install dependencies in parallel
            self.install_dependencies_parallel(&dependencies, "root", &mut progress)
                .await?;

            progress.finish();

            // Show summary
            println!(
                "\n{} Installed {} dependencies",
                style("✓").green().bold(),
                style(total_packages).white().bold()
            );
        } else {
            println!("{} No dependencies in package.json", style("•").yellow());
        }

        Ok(())
    }

    /// Uninstall a package from node_modules and package.json
    pub async fn uninstall_package(&self, package_name: &str) -> Result<()> {
        let package_dir = self.node_modules_dir.join(package_name);

        // Check if package is installed
        if !package_dir.exists() {
            println!(
                "{} {} is not installed",
                style("•").yellow(),
                style(package_name).white()
            );
            return Ok(());
        }

        // Check if other packages depend on this one
        let (can_remove, dependents) = self.check_can_remove_package(package_name, "root").await?;
        if !can_remove {
            println!(
                "{} Cannot remove {} - required by: {}",
                style("✗").red().bold(),
                style(package_name).white().bold(),
                style(dependents.join(", ")).dim()
            );
            return Ok(());
        }

        // Create progress tracker (simple for uninstall)
        let mut progress = ProgressTracker::new(1);
        progress
            .progress_bar
            .set_message(format!("{} {}", style("✗").red(), package_name));

        // Remove package directory
        fs::remove_dir_all(&package_dir).await?;

        // Get package info to check dependencies before removing
        let package_dependencies = self
            .get_package_dependencies_from_lock(package_name)
            .await?;

        // Update package.json to remove dependency
        self.remove_from_package_json(package_name).await?;

        // Update lock file and remove dependencies recursively
        self.remove_from_lock_file(package_name, "root").await?;

        // Remove dependencies if they're no longer needed
        for dep_name in package_dependencies {
            let (can_remove, _) = self
                .check_can_remove_package(&dep_name, package_name)
                .await?;
            if can_remove {
                // Remove dependency from filesystem
                let dep_dir = self.node_modules_dir.join(&dep_name);
                if dep_dir.exists() {
                    fs::remove_dir_all(&dep_dir).await?;
                }
                // Remove from lock file
                self.remove_from_lock_file(&dep_name, package_name).await?;
            }
        }

        // Update progress
        progress.update(&format!("{} Removed {}", style("✓").green(), package_name));
        progress.finish();

        // Show summary
        println!(
            "\n{} Uninstalled {}",
            style("✓").green().bold(),
            style(package_name).white().bold()
        );

        Ok(())
    }

    /// Ensure node_modules directory exists
    async fn ensure_node_modules_exists(&self) -> Result<()> {
        if !self.node_modules_dir.exists() {
            fs::create_dir_all(&self.node_modules_dir).await?;
        }
        Ok(())
    }

    /// Download package tarball to a temporary location
    async fn download_package_tarball(
        &self,
        package_info: &crate::package_info::PackageInfo,
    ) -> Result<PathBuf> {
        let tarball_filename = format!("{}-{}.tgz", package_info.name, package_info.version);

        // Create unique temp directory to avoid conflicts
        let temp_dir = PathBuf::from("temp").join(&package_info.name);
        let tarball_path = temp_dir.join(&tarball_filename);

        // Ensure temp directory exists
        fs::create_dir_all(&temp_dir).await?;

        self.npm_client
            .download_package(package_info, &tarball_path)
            .await?;
        Ok(tarball_path)
    }

    /// Extract package tarball to the specified directory
    async fn extract_package(&self, tarball_path: &Path, dest_dir: &Path) -> Result<()> {
        // Create the destination directory
        fs::create_dir_all(dest_dir).await?;

        // Use tar command to extract the tarball
        let output = Command::new("tar")
            .args([
                "-xzf",
                tarball_path.to_str().unwrap(),
                "-C",
                dest_dir.to_str().unwrap(),
                "--strip-components=1",
            ])
            .output()?;

        if !output.status.success() {
            let error_message = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Failed to extract tarball: {}", error_message));
        }

        Ok(())
    }

    /// Update or create package.json with the new dependency
    async fn update_package_json(&self, package_name: &str, version: &str) -> Result<()> {
        let _lock = self.file_mutex.lock().await;
        let mut package_json = if self.package_json_path.exists() {
            let content = fs::read_to_string(&self.package_json_path).await?;
            if content.trim().is_empty() {
                PackageJson::new()
            } else {
                serde_json::from_str::<PackageJson>(&content).unwrap_or_else(|_| PackageJson::new())
            }
        } else {
            PackageJson::new()
        };

        // Add the dependency
        package_json.add_dependency(package_name, version);

        // Write back to package.json
        let content = serde_json::to_string_pretty(&package_json)?;
        fs::write(&self.package_json_path, content).await?;

        Ok(())
    }

    /// Load or create lock file
    async fn load_lock_file(&self) -> Result<LockFile> {
        let _lock = self.file_mutex.lock().await;
        if self.lock_file_path.exists() {
            let content = fs::read_to_string(&self.lock_file_path).await?;
            if content.trim().is_empty() {
                Ok(LockFile::new())
            } else {
                let lock_file: LockFile =
                    serde_json::from_str(&content).unwrap_or_else(|_| LockFile::new());
                Ok(lock_file)
            }
        } else {
            Ok(LockFile::new())
        }
    }

    /// Save lock file
    async fn save_lock_file(&self, lock_file: &LockFile) -> Result<()> {
        let _lock = self.file_mutex.lock().await;
        let content = serde_json::to_string_pretty(lock_file)?;
        fs::write(&self.lock_file_path, content).await?;
        Ok(())
    }

    /// Update lock file with new package
    async fn update_lock_file(
        &self,
        name: &str,
        version: &str,
        resolved: &str,
        integrity: &str,
        dependencies: Option<&std::collections::HashMap<String, String>>,
        required_by: &str,
    ) -> Result<()> {
        let mut lock_file = self.load_lock_file().await?;
        lock_file.add_package(
            name,
            version,
            resolved,
            integrity,
            dependencies.cloned(),
            required_by,
        );
        self.save_lock_file(&lock_file).await?;
        Ok(())
    }

    /// Remove package from lock file
    async fn remove_from_lock_file(&self, name: &str, required_by: &str) -> Result<()> {
        let mut lock_file = self.load_lock_file().await?;
        lock_file.remove_package(name, required_by);
        self.save_lock_file(&lock_file).await?;
        Ok(())
    }

    /// Check if package can be removed
    async fn check_can_remove_package(
        &self,
        name: &str,
        required_by: &str,
    ) -> Result<(bool, Vec<String>)> {
        let lock_file = self.load_lock_file().await?;
        Ok(lock_file.can_remove_package(name, required_by))
    }

    /// Get dependencies of a package from lock file
    async fn get_package_dependencies_from_lock(&self, package_name: &str) -> Result<Vec<String>> {
        let lock_file = self.load_lock_file().await?;
        if let Some(package) = lock_file.packages.get(package_name) {
            if let Some(ref deps) = package.dependencies {
                return Ok(deps.keys().cloned().collect());
            }
        }
        Ok(Vec::new())
    }

    /// Remove a dependency from package.json
    async fn remove_from_package_json(&self, package_name: &str) -> Result<()> {
        let _lock = self.file_mutex.lock().await;
        if !self.package_json_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.package_json_path).await?;
        let mut package_json: PackageJson = if content.trim().is_empty() {
            PackageJson::new()
        } else {
            serde_json::from_str(&content).unwrap_or_else(|_| PackageJson::new())
        };

        // Remove from dependencies
        if let Some(ref mut deps) = package_json.dependencies {
            deps.remove(package_name);
        }

        // Write back to package.json
        let content = serde_json::to_string_pretty(&package_json)?;
        fs::write(&self.package_json_path, content).await?;

        Ok(())
    }

    /// List all installed packages with formatting
    pub async fn list_installed_packages(&self) -> Result<()> {
        if !self.node_modules_dir.exists() {
            println!("{} No packages installed", style("•").yellow());
            return Ok(());
        }

        let packages = self.get_installed_packages().await?;

        if packages.is_empty() {
            println!("{} No packages installed", style("•").yellow());
            return Ok(());
        }

        println!("{} Installed packages:", style("Packages").white().bold());
        for package in &packages {
            // Try to read package version from its package.json
            let version = self
                .get_package_version(package)
                .await
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "  {} {} {}",
                style("•").cyan(),
                style(package).white(),
                style(&format!("v{}", version)).dim()
            );
        }

        println!(
            "\n{} {} packages total",
            style("✓").green().bold(),
            style(packages.len()).white().bold()
        );

        Ok(())
    }

    /// Get list of installed package names
    async fn get_installed_packages(&self) -> Result<Vec<String>> {
        let mut packages = Vec::new();

        let mut entries = fs::read_dir(&self.node_modules_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    // Skip hidden directories and .bin
                    if !name.starts_with('.') {
                        packages.push(name.to_string());
                    }
                }
            }
        }

        packages.sort();
        Ok(packages)
    }

    /// Get version of an installed package
    async fn get_package_version(&self, package_name: &str) -> Option<String> {
        let package_json_path = self
            .node_modules_dir
            .join(package_name)
            .join("package.json");

        if let Ok(content) = fs::read_to_string(&package_json_path).await {
            if let Ok(package_json) = serde_json::from_str::<PackageJson>(&content) {
                return Some(package_json.version);
            }
        }

        None
    }

    /// Resolve version range to actual version by fetching from registry
    async fn resolve_version_range(
        &self,
        package_name: &str,
        version_range: &str,
    ) -> Result<String> {
        // For now, we'll use a simple approach:
        // - If it's already a specific version (x.y.z), use it as-is
        // - If it's a range (^x.y.z, ~x.y.z, *, etc.), fetch latest
        if Self::is_exact_version(version_range) {
            return Ok(version_range.to_string());
        }

        // For version ranges, fetch the latest version
        let registry_response = self.npm_client.get_package_info(package_name).await?;

        if let Some(package_info) = registry_response.get_latest_version() {
            Ok(package_info.version.clone())
        } else {
            Err(anyhow!(
                "Could not resolve version range '{}' for package '{}'",
                version_range,
                package_name
            ))
        }
    }
}

impl Default for PackageManager {
    fn default() -> Self {
        Self::new()
    }
}

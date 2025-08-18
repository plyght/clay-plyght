use anyhow::{Result, anyhow};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;

use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use crate::npm_client::NpmClient;
use crate::package_info::{DistInfo, LockFile, NpmRegistryResponse, PackageInfo, PackageJson};

#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub info: PackageInfo,
    pub dependencies: Vec<ResolvedPackage>,
    pub is_dev: bool,
}

pub struct PackageResolver {
    npm_client: NpmClient,
    resolved_cache: HashMap<String, NpmRegistryResponse>,
    resolution_stack: HashSet<String>,
}

impl PackageResolver {
    fn new(npm_client: NpmClient) -> Self {
        Self {
            npm_client,
            resolved_cache: HashMap::new(),
            resolution_stack: HashSet::new(),
        }
    }

    async fn resolve_package(
        &mut self,
        name: &str,
        version_spec: &str,
        is_dev: bool,
    ) -> Result<ResolvedPackage> {
        self.resolve_package_iterative(name, version_spec, is_dev)
            .await
    }

    async fn resolve_package_iterative(
        &mut self,
        root_name: &str,
        root_version_spec: &str,
        root_is_dev: bool,
    ) -> Result<ResolvedPackage> {
        use std::io::{self, Write};

        // Stack for iterative processing: (name, version_spec, is_dev, parent_path)
        let mut work_stack = vec![(
            root_name.to_string(),
            root_version_spec.to_string(),
            root_is_dev,
            String::new(),
        )];
        let mut resolved_packages: HashMap<String, ResolvedPackage> = HashMap::new();
        let mut dependency_graph: HashMap<String, Vec<String>> = HashMap::new();

        while let Some((name, version_spec, is_dev, _parent_path)) = work_stack.pop() {
            let package_key = format!("{}@{}", name, version_spec);

            // Check for circular dependency
            if self.resolution_stack.contains(&package_key) {
                continue;
            }

            // Skip if already resolved
            if resolved_packages.contains_key(&package_key) {
                continue;
            }

            self.resolution_stack.insert(package_key.clone());

            // Show intermediate resolution status
            print!(
                "\r    {} Fetching info for {}...{}",
                style("↓").blue(),
                style(&name).white(),
                " ".repeat(50)
            );
            io::stdout().flush().unwrap();

            // Fetch package info
            if !self.resolved_cache.contains_key(&name) {
                let response = self.npm_client.get_package_info(&name).await?;
                self.resolved_cache.insert(name.clone(), response);
            }
            let registry_response = self.resolved_cache.get(&name).unwrap();

            print!(
                "\r    {} Selecting version for {}...{}",
                style("🔍").yellow(),
                style(&name).white(),
                " ".repeat(50)
            );
            io::stdout().flush().unwrap();

            // Resolve version
            let package_info = if version_spec == "latest" {
                registry_response.get_latest_version()
            } else if Self::is_exact_version(&version_spec) {
                registry_response.get_version(&version_spec)
            } else {
                // For ranges, use latest for now
                registry_response.get_latest_version()
            }
            .ok_or_else(|| {
                anyhow!(
                    "Version '{}' not found for package '{}'",
                    version_spec,
                    name
                )
            })?;

            let package_info = package_info.clone();

            // Show dependency resolution status if package has dependencies
            if package_info.dependencies.is_some()
                && !package_info.dependencies.as_ref().unwrap().is_empty()
            {
                let dep_count = package_info.dependencies.as_ref().unwrap().len();
                print!(
                    "\r    {} Processing {} dependencies for {}...{}",
                    style("⚡").magenta(),
                    style(dep_count.to_string()).yellow(),
                    style(&name).white(),
                    " ".repeat(30)
                );
                io::stdout().flush().unwrap();
            }

            // Add dependencies to work stack
            let mut dep_keys = Vec::new();
            if let Some(ref deps) = package_info.dependencies {
                for (dep_name, dep_version) in deps {
                    let dep_key = format!("{}@{}", dep_name, dep_version);
                    dep_keys.push(dep_key.clone());
                    work_stack.push((
                        dep_name.clone(),
                        dep_version.clone(),
                        false,
                        package_key.clone(),
                    ));
                }
            }

            dependency_graph.insert(package_key.clone(), dep_keys);

            // Create resolved package with empty dependencies for now
            let resolved_pkg = ResolvedPackage {
                name: name.clone(),
                version: package_info.version.clone(),
                info: package_info,
                dependencies: Vec::new(), // Will be filled later
                is_dev,
            };

            resolved_packages.insert(package_key.clone(), resolved_pkg);
            self.resolution_stack.remove(&package_key);
        }

        // Build dependency tree
        print!(
            "\r    {} Building dependency tree for {}...{}",
            style("🌳").green(),
            style(root_name).white(),
            " ".repeat(50)
        );
        io::stdout().flush().unwrap();

        let root_key = format!("{}@{}", root_name, root_version_spec);
        let result = self.build_dependency_tree(&root_key, &resolved_packages, &dependency_graph);

        // Clear the building tree message completely
        print!("\r{}\r", " ".repeat(100));
        io::stdout().flush().unwrap();

        result
    }

    fn build_dependency_tree(
        &self,
        package_key: &str,
        resolved_packages: &HashMap<String, ResolvedPackage>,
        dependency_graph: &HashMap<String, Vec<String>>,
    ) -> Result<ResolvedPackage> {
        let mut visited = HashSet::new();
        self.build_tree_recursive(
            package_key,
            resolved_packages,
            dependency_graph,
            &mut visited,
        )
    }

    fn build_tree_recursive(
        &self,
        package_key: &str,
        resolved_packages: &HashMap<String, ResolvedPackage>,
        dependency_graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
    ) -> Result<ResolvedPackage> {
        if visited.contains(package_key) {
            // Return a stub for circular dependencies
            return Ok(ResolvedPackage {
                name: "circular".to_string(),
                version: "0.0.0".to_string(),
                info: PackageInfo {
                    name: "circular".to_string(),
                    version: "0.0.0".to_string(),
                    description: None,
                    main: None,
                    bin: None,
                    dist: DistInfo {
                        tarball: String::new(),
                        shasum: String::new(),
                    },
                    dependencies: None,
                },
                dependencies: Vec::new(),
                is_dev: false,
            });
        }

        visited.insert(package_key.to_string());

        let mut pkg = resolved_packages
            .get(package_key)
            .ok_or_else(|| anyhow!("Package not found: {}", package_key))?
            .clone();

        if let Some(dep_keys) = dependency_graph.get(package_key) {
            let mut dependencies = Vec::new();
            for dep_key in dep_keys {
                if let Ok(dep) =
                    self.build_tree_recursive(dep_key, resolved_packages, dependency_graph, visited)
                {
                    dependencies.push(dep);
                }
            }
            pkg.dependencies = dependencies;
        }

        visited.remove(package_key);
        Ok(pkg)
    }

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

    pub async fn resolve_multiple_packages(
        &mut self,
        packages: Vec<(String, String, bool)>, // name, version, is_dev
    ) -> Result<Vec<ResolvedPackage>> {
        let mut resolved = Vec::new();

        for (name, version, is_dev) in packages {
            print!(
                "\r  {} Resolving {}...{}",
                style("→").cyan(),
                style(&name).white(),
                " ".repeat(50)
            );
            use std::io::{self, Write};
            io::stdout().flush().unwrap();

            match self.resolve_package(&name, &version, is_dev).await {
                Ok(resolved_pkg) => {
                    print!(
                        "\r  {} Resolved {} ({}){}",
                        style("✓").green(),
                        style(&name).white(),
                        style(&resolved_pkg.version).dim(),
                        " ".repeat(50)
                    );
                    print!("\n");
                    io::stdout().flush().unwrap();
                    resolved.push(resolved_pkg);
                }
                Err(e) => {
                    print!("\r{}", " ".repeat(80));
                    print!(
                        "\r{} Failed to resolve {}: {}\n",
                        style("✗").red().bold(),
                        style(&name).white().bold(),
                        style(e.to_string()).dim()
                    );
                    io::stdout().flush().unwrap();
                    continue;
                }
            }
        }

        // Show resolution summary
        if !resolved.is_empty() {
            println!(
                "{} Resolved {} packages successfully",
                style("📦").green(),
                style(resolved.len().to_string()).green().bold()
            );
        }

        Ok(resolved)
    }

    pub fn count_total_packages(resolved: &[ResolvedPackage]) -> u64 {
        let mut count = 0;
        let mut visited = std::collections::HashSet::new();

        fn count_recursive(
            pkg: &ResolvedPackage,
            visited: &mut std::collections::HashSet<String>,
            count: &mut u64,
        ) {
            if !visited.insert(pkg.name.clone()) {
                return; // Already counted
            }
            *count += 1;
            for dep in &pkg.dependencies {
                count_recursive(dep, visited, count);
            }
        }

        for pkg in resolved {
            count_recursive(pkg, &mut visited, &mut count);
        }

        count
    }
}

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
    cache_dir: PathBuf,
    use_toml_lock: bool,
}

impl PackageManager {
    /// Create a new PackageManager with default settings
    pub fn new() -> Self {
        Self::with_toml_lock(true)
    }

    pub fn with_toml_lock(use_toml: bool) -> Self {
        let cache_dir = Self::get_cache_dir();
        let lock_file_path = if use_toml {
            PathBuf::from("clay-lock.toml")
        } else {
            PathBuf::from("clay-lock.json")
        };

        Self {
            npm_client: NpmClient::new(),
            node_modules_dir: PathBuf::from("node_modules"),
            package_json_path: PathBuf::from("package.json"),
            lock_file_path,
            semaphore: Arc::new(Semaphore::new(8)), // Limit concurrent downloads
            file_mutex: Arc::new(Mutex::new(())),
            cache_dir,
            use_toml_lock: use_toml,
        }
    }

    fn get_cache_dir() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(".clay").join("cache")
        } else {
            PathBuf::from(".clay-cache")
        }
    }

    async fn ensure_cache_dir_exists(&self) -> Result<()> {
        if !self.cache_dir.exists() {
            fs::create_dir_all(&self.cache_dir).await?;
        }
        Ok(())
    }

    fn get_cache_path(&self, package_info: &PackageInfo) -> PathBuf {
        self.cache_dir.join(format!(
            "{}@{}.tgz",
            package_info.name, package_info.version
        ))
    }

    async fn is_cached(&self, package_info: &PackageInfo) -> bool {
        let cache_path = self.get_cache_path(package_info);
        cache_path.exists()
    }

    async fn copy_from_cache(&self, package_info: &PackageInfo, dest_path: &Path) -> Result<()> {
        let cache_path = self.get_cache_path(package_info);
        if cache_path.exists() {
            fs::copy(&cache_path, dest_path).await?;

            // Verify cached file integrity
            let bytes = fs::read(dest_path).await?;
            if !self
                .npm_client
                .verify_package_integrity(&bytes, &package_info.dist.shasum)?
            {
                // Cache is corrupted, remove it
                fs::remove_file(&cache_path).await.ok();
                return Err(anyhow!("Cached file is corrupted"));
            }

            return Ok(());
        }
        Err(anyhow!("File not in cache"))
    }

    async fn save_to_cache(&self, package_info: &PackageInfo, source_path: &Path) -> Result<()> {
        self.ensure_cache_dir_exists().await?;
        let cache_path = self.get_cache_path(package_info);
        fs::copy(source_path, &cache_path).await?;
        Ok(())
    }

    /// Install multiple packages with unified progress
    pub async fn install_multiple_packages(
        &self,
        packages: Vec<(String, String)>,
        is_dev: bool,
        is_specific_install: bool,
    ) -> Result<()> {
        // Early check: see if all packages are already installed
        let (already_installed, packages_to_check) =
            self.check_packages_already_installed(&packages).await?;

        // Show already installed packages only for specific installs
        if is_specific_install {
            for package in &already_installed {
                println!(
                    "{} {} already installed",
                    style("•").cyan(),
                    style(package).white()
                );
            }
        }

        // If all packages are already installed, skip resolution entirely
        if packages_to_check.is_empty() {
            if is_specific_install {
                println!("{} All packages are already installed", style("✓").green());
            } else {
                println!("{} All packages are already installed", style("✓").green());
                self.show_installed_packages_summary().await?;
            }
            return Ok(());
        }

        let mut resolver = PackageResolver::new(self.npm_client.clone());
        let package_specs: Vec<(String, String, bool)> = packages_to_check
            .into_iter()
            .map(|(name, version)| (name, version, is_dev))
            .collect();

        // Phase 1: Resolution
        println!("{} Resolving dependencies...", style("🔍").blue());
        let resolved_packages = resolver.resolve_multiple_packages(package_specs).await?;

        if resolved_packages.is_empty() {
            println!("{} No valid packages to install", style("•").yellow());
            return Ok(());
        }

        // Check which resolved packages (including dependencies) are already installed
        let mut resolved_already_installed = Vec::new();
        let mut to_install = Vec::new();

        for resolved in &resolved_packages {
            let package_dir = self.node_modules_dir.join(&resolved.name);
            if package_dir.exists() {
                resolved_already_installed.push(resolved.name.clone());
            } else {
                to_install.push(resolved);
            }
        }

        // Show already installed dependencies only for specific installs
        if is_specific_install {
            for package in &resolved_already_installed {
                if !already_installed.contains(package) {
                    println!(
                        "{} {} already installed",
                        style("•").cyan(),
                        style(package).white()
                    );
                }
            }
        }

        if to_install.is_empty() {
            if is_specific_install {
                println!(
                    "{} All packages and dependencies are already installed",
                    style("✓").green()
                );
            } else {
                println!(
                    "{} All packages and dependencies are already installed",
                    style("✓").green()
                );
                self.show_installed_packages_summary().await?;
            }
            return Ok(());
        }

        // Phase 2: Count total packages (including dependencies)
        let total_packages = PackageResolver::count_total_packages(
            &to_install
                .iter()
                .map(|&pkg| pkg)
                .cloned()
                .collect::<Vec<_>>(),
        );

        let lock_format = if self.use_toml_lock { "TOML" } else { "JSON" };

        println!(
            "{} Installing {} packages (including {} dependencies) [{}]...",
            style("📦").green(),
            to_install.len(),
            total_packages - to_install.len() as u64,
            style(lock_format).dim()
        );

        // Phase 3: Install with progress tracking
        let mut progress = ProgressTracker::new(total_packages);

        for resolved_pkg in &to_install {
            self.install_resolved_package(resolved_pkg, true, &mut progress)
                .await?;
        }

        progress.finish();

        // Show summary
        if to_install.len() == 1 {
            println!(
                "\n{} Successfully installed {}",
                style("✓").green().bold(),
                style(&to_install[0].name).white().bold()
            );
        } else {
            println!(
                "\n{} Successfully installed {} packages",
                style("✓").green().bold(),
                style(to_install.len()).white().bold()
            );
        }

        // Show lock file format used
        let lock_format = if self.use_toml_lock { "TOML" } else { "JSON" };
        println!(
            "{} Lock file updated ({})",
            style("📄").blue(),
            style(lock_format).dim()
        );

        // Show summary of all installed packages only for package.json installs
        if !is_specific_install {
            self.show_installed_packages_summary().await?;
        }

        Ok(())
    }

    /// Install a package and save it to node_modules
    pub async fn install_package(&self, package_name: &str, version: &str) -> Result<()> {
        self.install_multiple_packages(
            vec![(package_name.to_string(), version.to_string())],
            false,
            true,
        )
        .await
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

    /// Install a resolved package with its dependencies
    async fn install_resolved_package(
        &self,
        resolved_pkg: &ResolvedPackage,
        update_package_json: bool,
        progress: &mut ProgressTracker,
    ) -> Result<()> {
        // Check if already installed
        let package_dir = self.node_modules_dir.join(&resolved_pkg.name);
        if package_dir.exists() {
            progress.update(&format!(
                "{} {} (cached)",
                style("•").cyan(),
                resolved_pkg.name
            ));
            return Ok(());
        }

        // Install dependencies first
        for dep in &resolved_pkg.dependencies {
            Box::pin(self.install_resolved_package(dep, false, progress)).await?;
        }

        // Install this package
        self.install_single_package(
            &resolved_pkg.info,
            update_package_json,
            resolved_pkg.is_dev,
            progress,
        )
        .await?;

        Ok(())
    }

    /// Install a single package without dependency resolution
    async fn install_single_package(
        &self,
        package_info: &PackageInfo,
        update_package_json: bool,
        is_dev: bool,
        progress: &mut ProgressTracker,
    ) -> Result<()> {
        // Skip circular dependency stubs
        if package_info.name == "circular" {
            return Ok(());
        }

        // Ensure node_modules directory exists
        self.ensure_node_modules_exists().await?;

        // Check if package is already installed
        let package_dir = self.node_modules_dir.join(&package_info.name);
        if package_dir.exists() {
            return Ok(());
        }

        // Download the package tarball
        progress.update(&format!("{} {}", style("↓").cyan(), package_info.name));
        let tarball_path = self.download_package_tarball(package_info).await?;

        // Check if tarball was actually created
        if !tarball_path.exists() {
            return Err(anyhow!(
                "Failed to download tarball for {}",
                package_info.name
            ));
        }

        // Extract the tarball to node_modules
        progress.update(&format!("{} {}", style("⚡").yellow(), package_info.name));
        self.extract_package(&tarball_path, &package_dir).await?;

        // Setup bin commands for this package
        self.setup_bin_commands(&package_info.name, &package_dir)
            .await?;

        // Clean up the tarball and temp directory
        if tarball_path.exists() {
            fs::remove_file(&tarball_path).await.ok();
        }
        if let Some(temp_dir) = tarball_path.parent() {
            fs::remove_dir_all(temp_dir).await.ok();
        }

        // Update package.json only if this is the explicitly requested package
        if update_package_json {
            self.update_package_json(&package_info.name, &package_info.version, is_dev)
                .await?;
        }

        // Update lock file
        let parent_name = if update_package_json {
            "root"
        } else {
            // For dependency packages, use the package name as parent
            &package_info.name
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

        let mut total_packages = 0;
        let mut has_deps = false;

        // Count regular dependencies
        if let Some(dependencies) = &package_json.dependencies {
            if !dependencies.is_empty() {
                has_deps = true;
                for (dep_name, _) in dependencies {
                    let dep_package_dir = self.node_modules_dir.join(dep_name);
                    if !dep_package_dir.exists() {
                        total_packages += 1;
                    }
                }
            }
        }

        // Count dev dependencies
        if let Some(dev_dependencies) = &package_json.dev_dependencies {
            if !dev_dependencies.is_empty() {
                has_deps = true;
                for (dep_name, _) in dev_dependencies {
                    let dep_package_dir = self.node_modules_dir.join(dep_name);
                    if !dep_package_dir.exists() {
                        total_packages += 1;
                    }
                }
            }
        }

        if !has_deps {
            println!("{} No dependencies in package.json", style("•").yellow());
            return Ok(());
        }

        if total_packages == 0 {
            println!("{} All dependencies already installed", style("✓").green());
            return Ok(());
        }

        // Create progress tracker
        let mut progress = ProgressTracker::new(total_packages);

        // Install regular dependencies
        if let Some(dependencies) = package_json.dependencies {
            self.install_dependencies_parallel(&dependencies, "root", &mut progress)
                .await?;
        }

        // Install dev dependencies
        if let Some(dev_dependencies) = package_json.dev_dependencies {
            self.install_dependencies_parallel(&dev_dependencies, "root", &mut progress)
                .await?;
        }

        progress.finish();

        // Show summary
        println!(
            "\n{} Installed {} dependencies",
            style("✓").green().bold(),
            style(total_packages).white().bold()
        );

        Ok(())
    }

    /// Uninstall a package from node_modules and package.json
    pub async fn uninstall_package(&self, package_name: &str) -> Result<()> {
        let package_dir = self.node_modules_dir.join(package_name);

        // Check if package is installed
        if !package_dir.exists() {
            println!(
                "{} {} is not installed",
                style("•").dim(),
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

        // Cleanup bin commands before removing package
        self.cleanup_bin_commands(package_name).await?;

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
                // Cleanup bin commands for dependency
                self.cleanup_bin_commands(&dep_name).await?;

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

    /// Download package tarball to a temporary location (with caching)
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

        // Try to copy from cache first
        if self.is_cached(package_info).await {
            match self.copy_from_cache(package_info, &tarball_path).await {
                Ok(()) => {
                    return Ok(tarball_path);
                }
                Err(_) => {
                    // Cache miss or corrupted, continue with download
                }
            }
        }

        // Download from registry
        self.npm_client
            .download_package(package_info, &tarball_path)
            .await?;

        // Save to cache for future use
        self.save_to_cache(package_info, &tarball_path).await.ok();

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
    async fn update_package_json(
        &self,
        package_name: &str,
        version: &str,
        is_dev: bool,
    ) -> Result<()> {
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
        if is_dev {
            package_json.add_dev_dependency(package_name, version);
        } else {
            package_json.add_dependency(package_name, version);
        }

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
                let lock_file: LockFile = if self.use_toml_lock {
                    toml::from_str(&content).unwrap_or_else(|_| LockFile::new())
                } else {
                    serde_json::from_str(&content).unwrap_or_else(|_| LockFile::new())
                };
                Ok(lock_file)
            }
        } else {
            Ok(LockFile::new())
        }
    }

    /// Save lock file
    async fn save_lock_file(&self, lock_file: &LockFile) -> Result<()> {
        let _lock = self.file_mutex.lock().await;
        let content = if self.use_toml_lock {
            toml::to_string_pretty(lock_file)?
        } else {
            serde_json::to_string_pretty(lock_file)?
        };
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

        let user_packages = self.get_user_installed_packages().await?;
        let all_packages = self.get_installed_packages().await?;

        if all_packages.is_empty() {
            println!("{} No packages installed", style("•").yellow());
            return Ok(());
        }

        // Show user-installed packages
        if !user_packages.is_empty() {
            println!("{} User-installed packages:", style("📦").blue().bold());
            for package in &user_packages {
                let version = self
                    .get_package_version(package)
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                println!(
                    "  {} {} {}",
                    style("•").cyan(),
                    style(package).white().bold(),
                    style(&format!("v{}", version)).dim()
                );
            }
            println!();
        }

        // Show dependencies (packages not in package.json)
        let dependencies: Vec<_> = all_packages
            .iter()
            .filter(|pkg| !user_packages.contains(pkg))
            .collect();

        if !dependencies.is_empty() {
            println!("{} Dependencies:", style("🔗").dim().bold());
            for package in &dependencies {
                let version = self
                    .get_package_version(package)
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                println!(
                    "  {} {} {}",
                    style("•").dim(),
                    style(package).dim(),
                    style(&format!("v{}", version)).dim()
                );
            }
            println!();
        }

        println!(
            "{} {} user packages, {} dependencies ({} total)",
            style("✓").green().bold(),
            style(user_packages.len()).white().bold(),
            style(dependencies.len()).dim(),
            style(all_packages.len()).white()
        );

        Ok(())
    }

    async fn show_installed_packages_summary(&self) -> Result<()> {
        if !self.node_modules_dir.exists() {
            return Ok(());
        }

        // Get user-installed packages (from package.json)
        let user_packages = self.get_user_installed_packages().await?;

        if user_packages.is_empty() {
            return Ok(());
        }

        println!("\n{} Installed packages:", style("📋").blue());

        // Show packages in a more compact format
        let mut current_line = String::new();
        for package in user_packages.iter() {
            let version = self
                .get_package_version(package)
                .await
                .unwrap_or_else(|| "unknown".to_string());

            let package_str = format!("{}@{}", package, version);

            if current_line.is_empty() {
                current_line = format!("  {}", package_str);
            } else if current_line.len() + package_str.len() + 2 < 80 {
                current_line.push_str(&format!(", {}", package_str));
            } else {
                println!("{}", current_line);
                current_line = format!("  {}", package_str);
            }
        }

        if !current_line.is_empty() {
            println!("{}", current_line);
        }

        println!(
            "\n{} {} packages total",
            style("✓").green().bold(),
            style(user_packages.len()).white().bold()
        );

        Ok(())
    }

    async fn get_user_installed_packages(&self) -> Result<Vec<String>> {
        let mut user_packages = Vec::new();

        // Read package.json to get user-installed packages
        if self.package_json_path.exists() {
            let content = fs::read_to_string(&self.package_json_path).await?;
            if !content.trim().is_empty() {
                if let Ok(package_json) = serde_json::from_str::<PackageJson>(&content) {
                    // Add regular dependencies
                    if let Some(dependencies) = &package_json.dependencies {
                        for name in dependencies.keys() {
                            let package_dir = self.node_modules_dir.join(name);
                            if package_dir.exists() {
                                user_packages.push(name.clone());
                            }
                        }
                    }

                    // Add dev dependencies
                    if let Some(dev_dependencies) = &package_json.dev_dependencies {
                        for name in dev_dependencies.keys() {
                            let package_dir = self.node_modules_dir.join(name);
                            if package_dir.exists() {
                                user_packages.push(name.clone());
                            }
                        }
                    }
                }
            }
        }

        user_packages.sort();
        Ok(user_packages)
    }

    async fn check_packages_already_installed(
        &self,
        package_specs: &[(String, String)],
    ) -> Result<(Vec<String>, Vec<(String, String)>)> {
        let mut already_installed = Vec::new();
        let mut to_install = Vec::new();

        for (name, version) in package_specs {
            let package_dir = self.node_modules_dir.join(name);
            if package_dir.exists() {
                already_installed.push(name.clone());
            } else {
                to_install.push((name.clone(), version.clone()));
            }
        }

        Ok((already_installed, to_install))
    }

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
                return package_json.version;
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

    /// Show cache information
    pub async fn cache_info(&self) -> Result<()> {
        use console::style;

        self.ensure_cache_dir_exists().await?;

        let mut total_size = 0u64;
        let mut package_count = 0u32;

        if self.cache_dir.exists() {
            let mut entries = fs::read_dir(&self.cache_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(metadata) = entry.metadata().await {
                    if metadata.is_file()
                        && entry.path().extension().map_or(false, |ext| ext == "tgz")
                    {
                        total_size += metadata.len();
                        package_count += 1;
                    }
                }
            }
        }

        println!("{} Cache Information", style("📦").cyan());
        println!("Cache directory: {}", style(self.cache_dir.display()).dim());
        println!(
            "Cached packages: {}",
            style(package_count.to_string()).green()
        );
        println!(
            "Total size: {}",
            style(Self::format_size(total_size)).green()
        );

        Ok(())
    }

    /// Clear all cached packages
    pub async fn cache_clear(&self) -> Result<()> {
        use console::style;

        if self.cache_dir.exists() {
            let mut entries = fs::read_dir(&self.cache_dir).await?;
            let mut cleared_count = 0u32;

            while let Some(entry) = entries.next_entry().await? {
                if entry.path().extension().map_or(false, |ext| ext == "tgz") {
                    fs::remove_file(entry.path()).await?;
                    cleared_count += 1;
                }
            }

            println!(
                "{} Cleared {} cached packages",
                style("✓").green(),
                style(cleared_count.to_string()).green()
            );
        } else {
            println!("{} Cache directory does not exist", style("•").yellow());
        }

        Ok(())
    }

    /// Show cache directory path
    pub async fn cache_dir(&self) -> Result<()> {
        use console::style;

        println!("{}", self.cache_dir.display());

        if !self.cache_dir.exists() {
            println!("{} Cache directory does not exist yet", style("•").dim());
        }

        Ok(())
    }

    fn format_size(bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size as u64, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }

    /// Read dependencies from package.json and convert to package specs
    pub async fn get_package_json_dependencies(
        &self,
        include_dev: bool,
    ) -> Result<Vec<(String, String)>> {
        if !self.package_json_path.exists() {
            println!("{} No package.json found", style("•").yellow());
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.package_json_path).await?;
        let package_json: PackageJson = if content.trim().is_empty() {
            PackageJson::new()
        } else {
            serde_json::from_str(&content).unwrap_or_else(|_| PackageJson::new())
        };

        let mut package_specs = Vec::new();

        // Add regular dependencies
        if let Some(dependencies) = &package_json.dependencies {
            for (name, version_spec) in dependencies {
                package_specs.push((name.clone(), version_spec.clone()));
            }
        }

        // Add dev dependencies if requested
        if include_dev {
            if let Some(dev_dependencies) = &package_json.dev_dependencies {
                for (name, version_spec) in dev_dependencies {
                    package_specs.push((name.clone(), version_spec.clone()));
                }
            }
        }

        if package_specs.is_empty() {
            println!(
                "{} No dependencies found in package.json",
                style("•").yellow()
            );
        }

        Ok(package_specs)
    }

    async fn setup_bin_commands(&self, package_name: &str, package_dir: &Path) -> Result<()> {
        // Read the package's package.json to get bin information
        let package_json_path = package_dir.join("package.json");
        if !package_json_path.exists() {
            return Ok(());
        }

        let content = match fs::read_to_string(&package_json_path).await {
            Ok(content) => content,
            Err(_) => return Ok(()), // Skip if can't read package.json
        };

        let package_json: Value = match serde_json::from_str(&content) {
            Ok(json) => json,
            Err(_) => return Ok(()), // Skip if invalid JSON
        };

        if let Some(bin) = package_json.get("bin") {
            let bin_dir = self.node_modules_dir.join(".bin");
            if let Err(e) = fs::create_dir_all(&bin_dir).await {
                eprintln!(
                    "{} Failed to create .bin directory: {}",
                    style("⚠").yellow(),
                    e
                );
                return Ok(());
            }

            match bin {
                // Handle string format: "bin": "path/to/executable"
                Value::String(bin_path) => {
                    let executable_name = package_name;
                    if let Err(e) = self
                        .create_bin_link(
                            executable_name,
                            package_name,
                            bin_path,
                            &bin_dir,
                            package_dir,
                        )
                        .await
                    {
                        eprintln!(
                            "{} Failed to create bin command {}: {}",
                            style("⚠").yellow(),
                            style(executable_name).white(),
                            e
                        );
                    }
                }
                // Handle object format: "bin": { "command": "path/to/executable" }
                Value::Object(bin_map) => {
                    for (command_name, bin_path) in bin_map {
                        if let Value::String(path_str) = bin_path {
                            if let Err(e) = self
                                .create_bin_link(
                                    command_name,
                                    package_name,
                                    path_str,
                                    &bin_dir,
                                    package_dir,
                                )
                                .await
                            {
                                eprintln!(
                                    "{} Failed to create bin command {}: {}",
                                    style("⚠").yellow(),
                                    style(command_name).white(),
                                    e
                                );
                            } else {
                                // println!(
                                //     "{} Added bin command: {}",
                                //     style("🔧").blue(),
                                //     style(command_name).white()
                                // );
                            }
                        }
                    }
                }
                _ => {} // Invalid bin format, skip
            }
        }

        Ok(())
    }

    async fn create_bin_link(
        &self,
        command_name: &str,
        _package_name: &str,
        bin_path: &str,
        bin_dir: &Path,
        package_dir: &Path,
    ) -> Result<()> {
        let source_path = package_dir.join(bin_path);
        let link_path = bin_dir.join(command_name);

        // Remove existing link if it exists
        if link_path.exists() {
            fs::remove_file(&link_path).await.ok();
        }

        #[cfg(unix)]
        {
            // On Unix systems, create a symlink and make source executable
            use std::os::unix::fs as unix_fs;
            use std::os::unix::fs::PermissionsExt;

            // Make the source file executable if it isn't already
            if source_path.exists() {
                if let Ok(metadata) = fs::metadata(&source_path).await {
                    let mut perms = metadata.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    let _ = fs::set_permissions(&source_path, perms).await;
                }
            }

            unix_fs::symlink(&source_path, &link_path)?;
        }

        #[cfg(windows)]
        {
            // On Windows, create a batch file that calls the executable
            let batch_content = format!(
                "@echo off\nnode \"{}\" %*",
                source_path.to_string_lossy().replace('/', "\\")
            );
            let batch_path = bin_dir.join(format!("{}.cmd", command_name));
            fs::write(&batch_path, batch_content).await?;
        }

        Ok(())
    }

    async fn cleanup_bin_commands(&self, package_name: &str) -> Result<()> {
        let bin_dir = self.node_modules_dir.join(".bin");
        if !bin_dir.exists() {
            return Ok(());
        }

        // Get the package's package.json to know which bin commands to remove
        let package_dir = self.node_modules_dir.join(package_name);
        let package_json_path = package_dir.join("package.json");

        if package_json_path.exists() {
            let content = fs::read_to_string(&package_json_path).await?;
            if let Ok(package_json) = serde_json::from_str::<Value>(&content) {
                if let Some(bin) = package_json.get("bin") {
                    match bin {
                        Value::String(_) => {
                            let link_path = bin_dir.join(package_name);
                            if link_path.exists() {
                                fs::remove_file(&link_path).await.ok();
                                println!(
                                    "{} Removed bin command: {}",
                                    style("🔧").dim(),
                                    style(package_name).dim()
                                );
                            }
                            #[cfg(windows)]
                            {
                                let batch_path = bin_dir.join(format!("{}.cmd", package_name));
                                if batch_path.exists() {
                                    fs::remove_file(&batch_path).await.ok();
                                }
                            }
                        }
                        Value::Object(bin_map) => {
                            for command_name in bin_map.keys() {
                                let link_path = bin_dir.join(command_name);
                                if link_path.exists() {
                                    fs::remove_file(&link_path).await.ok();
                                    println!(
                                        "{} Removed bin command: {}",
                                        style("🔧").dim(),
                                        style(command_name).dim()
                                    );
                                }
                                #[cfg(windows)]
                                {
                                    let batch_path = bin_dir.join(format!("{}.cmd", command_name));
                                    if batch_path.exists() {
                                        fs::remove_file(&batch_path).await.ok();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    /// Run a script from package.json
    pub async fn run_script(&self, script_name: &str) -> Result<()> {
        // Check if package.json exists
        if !self.package_json_path.exists() {
            println!("{} No package.json found", style("✗").red());
            return Ok(());
        }

        // Read package.json
        let content = fs::read_to_string(&self.package_json_path).await?;
        let package_json: Value = serde_json::from_str(&content)?;

        // Get scripts section
        let scripts = match package_json.get("scripts") {
            Some(Value::Object(scripts)) => scripts,
            _ => {
                println!("{} No scripts found in package.json", style("✗").red());
                return Ok(());
            }
        };

        // Find the requested script
        let script_command = match scripts.get(script_name) {
            Some(Value::String(command)) => command,
            _ => {
                println!(
                    "{} Script '{}' not found",
                    style("✗").red(),
                    style(script_name).white()
                );

                // Show available scripts
                if !scripts.is_empty() {
                    println!("\n{} Available scripts:", style("Scripts").blue().bold());
                    for (name, command) in scripts {
                        if let Value::String(cmd) = command {
                            println!(
                                "  {} {} {}",
                                style("•").cyan(),
                                style(name).white().bold(),
                                style(cmd).dim()
                            );
                        }
                    }
                }
                return Ok(());
            }
        };

        println!(
            "{} Running script: {} {}",
            style("🚀").blue(),
            style(script_name).white().bold(),
            style(&format!("({})", script_command)).dim()
        );

        // Set up environment with .bin in PATH
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

        // Add node_modules/.bin to PATH
        let bin_dir = self.node_modules_dir.join(".bin");
        if bin_dir.exists() {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let path_separator = if cfg!(target_os = "windows") {
                ";"
            } else {
                ":"
            };
            let new_path = if current_path.is_empty() {
                bin_dir.to_string_lossy().to_string()
            } else {
                format!(
                    "{}{}{}",
                    bin_dir.to_string_lossy(),
                    path_separator,
                    current_path
                )
            };
            cmd.env("PATH", new_path);
        }

        // Set working directory to project root
        cmd.current_dir(self.package_json_path.parent().unwrap_or(Path::new(".")));

        // Execute the command
        let status = cmd.status()?;

        if status.success() {
            println!(
                "\n{} Script '{}' completed successfully",
                style("✓").green().bold(),
                style(script_name).white()
            );
        } else {
            println!(
                "\n{} Script '{}' failed with exit code: {}",
                style("✗").red().bold(),
                style(script_name).white(),
                status.code().unwrap_or(-1)
            );
        }

        Ok(())
    }

    /// List all available scripts from package.json
    pub async fn list_scripts(&self) -> Result<()> {
        // Check if package.json exists
        if !self.package_json_path.exists() {
            println!("{} No package.json found", style("✗").red());
            return Ok(());
        }

        // Read package.json
        let content = fs::read_to_string(&self.package_json_path).await?;
        let package_json: Value = serde_json::from_str(&content)?;

        // Get scripts section
        let scripts = match package_json.get("scripts") {
            Some(Value::Object(scripts)) => scripts,
            _ => {
                println!("{} No scripts found in package.json", style("•").yellow());
                return Ok(());
            }
        };

        if scripts.is_empty() {
            println!("{} No scripts found in package.json", style("•").yellow());
            return Ok(());
        }

        println!("{} Available scripts:", style("Scripts").blue().bold());

        // Sort scripts by name for consistent output
        let mut sorted_scripts: Vec<_> = scripts.iter().collect();
        sorted_scripts.sort_by_key(|(name, _)| *name);

        for (name, command) in sorted_scripts {
            if let Value::String(cmd) = command {
                println!(
                    "  {} {} {}",
                    style("•").cyan(),
                    style(name).white().bold(),
                    style(cmd).dim()
                );
            }
        }

        println!(
            "\n{} Run a script with: {} {}",
            style("💡").yellow(),
            style("clay run").cyan(),
            style("<script-name>").dim()
        );

        Ok(())
    }
}

impl Default for PackageManager {
    fn default() -> Self {
        Self::new()
    }
}

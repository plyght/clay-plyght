use anyhow::Result;
use console::style;

use crate::cli_style::CliStyle;
use dashmap::DashMap;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::Archive;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAddress {
    pub hash: String,
    pub size: u64,
    pub integrity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    pub name: String,
    pub version: String,
    pub content_address: ContentAddress,
    pub dependencies: Option<HashMap<String, String>>,
    pub files: Vec<String>,
}

pub struct ContentStore {
    store_path: PathBuf,
    index: Arc<DashMap<String, ContentAddress>>,
    package_index: Arc<DashMap<String, PackageMetadata>>,
}

impl ContentStore {
    pub fn new() -> Self {
        let store_path = Self::get_store_path();
        Self {
            store_path,
            index: Arc::new(DashMap::new()),
            package_index: Arc::new(DashMap::new()),
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        self.ensure_store_directories().await?;
        self.load_index().await?;
        Ok(())
    }

    pub async fn store_package(
        &self,
        package_name: &str,
        package_version: &str,
        tarball_data: &[u8],
        integrity_hash: &str,
    ) -> Result<ContentAddress> {
        // Calculate content hash
        let content_hash = self.calculate_content_hash(tarball_data);
        let content_address = ContentAddress {
            hash: content_hash.clone(),
            size: tarball_data.len() as u64,
            integrity: integrity_hash.to_string(),
        };

        // Check if content already exists
        if let Some(existing) = self.index.get(&content_hash) {
            println!(
                "{} Package {} already exists in content store ({})",
                CliStyle::cyan_text(""),
                style(package_name).white(),
                style(&existing.hash[..8]).dim()
            );
            return Ok(existing.clone());
        }

        // Store the content
        let content_path = self.get_content_path(&content_hash);
        if let Some(parent) = content_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Compress and store the tarball
        let compressed_data = self.compress_data(tarball_data)?;
        fs::write(&content_path, &compressed_data).await?;

        // Extract and analyze package contents
        let package_metadata = self
            .analyze_package_content(
                package_name,
                package_version,
                tarball_data,
                content_address.clone(),
            )
            .await?;

        // Update indices
        self.index
            .insert(content_hash.clone(), content_address.clone());
        let package_key = format!("{package_name}@{package_version}");
        self.package_index.insert(package_key, package_metadata);

        // Persist index
        self.save_index().await?;

        println!(
            "{} Stored {} in content store ({})",
            style("💾").green(),
            style(package_name).white(),
            style(&content_hash[..8]).dim()
        );

        Ok(content_address)
    }

    pub async fn link_package(
        &self,
        package_name: &str,
        package_version: &str,
        target_path: &Path,
    ) -> Result<bool> {
        let package_key = format!("{package_name}@{package_version}");

        if let Some(metadata) = self.package_index.get(&package_key) {
            let content_path = self.get_content_path(&metadata.content_address.hash);

            if content_path.exists() {
                // Create target directory
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent).await?;
                }

                // Extract package to target location
                self.extract_package_from_store(&content_path, target_path)
                    .await?;

                println!(
                    "{} Linked {} from content store ({})",
                    CliStyle::info(""),
                    style(package_name).white(),
                    style(&metadata.content_address.hash[..8]).dim()
                );

                return Ok(true);
            }
        }

        Ok(false)
    }

    pub async fn get_package_info(
        &self,
        package_name: &str,
        package_version: &str,
    ) -> Option<PackageMetadata> {
        let package_key = format!("{package_name}@{package_version}");
        self.package_index
            .get(&package_key)
            .map(|entry| entry.clone())
    }

    pub async fn deduplicate_store(&self) -> Result<u64> {
        let dedup_spinner =
            CliStyle::create_spinner("Analyzing content store for deduplication...");

        let mut saved_bytes = 0u64;
        let mut duplicate_count = 0u32;

        // Group packages by content hash
        let mut content_groups: HashMap<String, Vec<String>> = HashMap::new();

        for entry in self.package_index.iter() {
            let package_key = entry.key().clone();
            let metadata = entry.value();

            content_groups
                .entry(metadata.content_address.hash.clone())
                .or_default()
                .push(package_key);
        }

        // Report deduplication statistics
        for (content_hash, packages) in content_groups {
            if packages.len() > 1 {
                duplicate_count += packages.len() as u32 - 1;

                if let Some(metadata) = self.package_index.get(&packages[0]) {
                    saved_bytes += metadata.content_address.size * (packages.len() as u64 - 1);
                }

                println!(
                    "{} Content {} shared by {} packages: {}",
                    CliStyle::cyan_text(""),
                    style(&content_hash[..8]).dim(),
                    style(packages.len()).green(),
                    packages.join(", ")
                );
            }
        }

        if duplicate_count > 0 {
            dedup_spinner.finish_with_message(format!(
                "Deduplication saved {} ({} duplicate packages)",
                Self::format_size(saved_bytes),
                duplicate_count
            ));
        } else {
            dedup_spinner.finish_with_message("No duplicates found in content store");
        }

        Ok(saved_bytes)
    }

    pub async fn cleanup_unused(&self, active_packages: &[String]) -> Result<u64> {
        let cleanup_spinner =
            CliStyle::create_spinner("Cleaning up unused packages from content store...");

        let active_set: std::collections::HashSet<_> = active_packages.iter().collect();
        let mut removed_bytes = 0u64;
        let mut removed_count = 0u32;

        // Find packages to remove
        let mut to_remove = Vec::new();
        for entry in self.package_index.iter() {
            if !active_set.contains(entry.key()) {
                to_remove.push((entry.key().clone(), entry.value().clone()));
            }
        }

        // Remove unused packages
        for (package_key, metadata) in to_remove {
            let content_path = self.get_content_path(&metadata.content_address.hash);

            if content_path.exists() {
                fs::remove_file(&content_path).await?;
                removed_bytes += metadata.content_address.size;
                removed_count += 1;
            }

            self.package_index.remove(&package_key);
        }

        // Clean up orphaned content
        let mut content_refs: HashMap<String, u32> = HashMap::new();
        for entry in self.package_index.iter() {
            let hash = &entry.value().content_address.hash;
            *content_refs.entry(hash.clone()).or_insert(0) += 1;
        }

        for entry in self.index.iter() {
            if !content_refs.contains_key(entry.key()) {
                let content_path = self.get_content_path(entry.key());
                if content_path.exists() {
                    fs::remove_file(&content_path).await?;
                    removed_bytes += entry.value().size;
                }
            }
        }

        // Update index
        self.index.retain(|hash, _| content_refs.contains_key(hash));
        self.save_index().await?;

        if removed_count > 0 {
            cleanup_spinner.finish_with_message(format!(
                "Cleaned up {} packages ({} freed)",
                removed_count,
                Self::format_size(removed_bytes)
            ));
        } else {
            cleanup_spinner.finish_with_message("No unused packages found");
        }

        Ok(removed_bytes)
    }

    pub async fn get_store_stats(&self) -> Result<StoreStats> {
        let mut total_content_size = 0u64;
        let mut duplicates = 0u32;

        // Count packages
        let total_packages = self.package_index.len() as u32;

        // Count unique content
        let unique_content_count = self.index.len() as u32;

        // Calculate total size and duplicates
        let mut content_usage: HashMap<String, u32> = HashMap::new();
        for entry in self.package_index.iter() {
            let hash = &entry.value().content_address.hash;
            *content_usage.entry(hash.clone()).or_insert(0) += 1;
        }

        for entry in self.index.iter() {
            total_content_size += entry.value().size;
            if let Some(usage) = content_usage.get(entry.key()) {
                if *usage > 1 {
                    duplicates += usage - 1;
                }
            }
        }

        Ok(StoreStats {
            total_packages,
            unique_content_count,
            total_content_size,
            duplicate_packages: duplicates,
            space_saved: self.calculate_space_saved().await?,
        })
    }

    async fn calculate_space_saved(&self) -> Result<u64> {
        let mut total_if_duplicated = 0u64;
        let mut content_usage: HashMap<String, u32> = HashMap::new();

        for entry in self.package_index.iter() {
            let hash = &entry.value().content_address.hash;
            *content_usage.entry(hash.clone()).or_insert(0) += 1;
        }

        for (hash, usage) in content_usage {
            if let Some(content) = self.index.get(&hash) {
                total_if_duplicated += content.size * usage as u64;
            }
        }

        let actual_size: u64 = self.index.iter().map(|entry| entry.value().size).sum();
        Ok(total_if_duplicated.saturating_sub(actual_size))
    }

    fn get_store_path() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(".clay").join("content-store")
        } else {
            PathBuf::from(".clay-content-store")
        }
    }

    async fn ensure_store_directories(&self) -> Result<()> {
        fs::create_dir_all(&self.store_path).await?;
        fs::create_dir_all(self.store_path.join("content")).await?;
        fs::create_dir_all(self.store_path.join("index")).await?;
        Ok(())
    }

    fn get_content_path(&self, content_hash: &str) -> PathBuf {
        // Use first 2 chars for directory sharding
        let dir = &content_hash[..2];
        let file = &content_hash[2..];
        self.store_path
            .join("content")
            .join(dir)
            .join(format!("{file}.tar.gz"))
    }

    fn calculate_content_hash(&self, data: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    fn compress_data(&self, data: &[u8]) -> Result<Vec<u8>> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        Ok(encoder.finish()?)
    }

    async fn analyze_package_content(
        &self,
        package_name: &str,
        package_version: &str,
        tarball_data: &[u8],
        content_address: ContentAddress,
    ) -> Result<PackageMetadata> {
        let mut files = Vec::new();
        let mut dependencies = None;

        // Extract and analyze tarball
        let decoder = GzDecoder::new(tarball_data);
        let mut archive = Archive::new(decoder);

        for entry in archive.entries()? {
            let entry = entry?;
            if let Ok(path) = entry.path() {
                let path_str = path.to_string_lossy().to_string();
                files.push(path_str.clone());

                // Parse package.json if present
                if path_str.ends_with("package.json") {
                    let mut contents = Vec::new();
                    let mut entry = entry;
                    entry.read_to_end(&mut contents)?;

                    if let Ok(package_json) = serde_json::from_slice::<serde_json::Value>(&contents)
                    {
                        if let Some(deps) = package_json.get("dependencies") {
                            if let Ok(deps_map) =
                                serde_json::from_value::<HashMap<String, String>>(deps.clone())
                            {
                                dependencies = Some(deps_map);
                            }
                        }
                    }
                }
            }
        }

        Ok(PackageMetadata {
            name: package_name.to_string(),
            version: package_version.to_string(),
            content_address,
            dependencies,
            files,
        })
    }

    async fn extract_package_from_store(
        &self,
        store_path: &Path,
        target_path: &Path,
    ) -> Result<()> {
        // Read compressed data
        let compressed_data = fs::read(store_path).await?;

        // Decompress
        use flate2::read::GzDecoder;
        let mut decoder = GzDecoder::new(&compressed_data[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        // Extract tarball
        let mut archive = Archive::new(&decompressed[..]);
        archive.set_overwrite(true);
        archive.unpack(target_path)?;

        Ok(())
    }

    async fn load_index(&self) -> Result<()> {
        let index_path = self.store_path.join("index").join("content.json");
        let package_index_path = self.store_path.join("index").join("packages.json");

        // Load content index
        if index_path.exists() {
            let content = fs::read_to_string(&index_path).await?;
            if let Ok(index_data) =
                serde_json::from_str::<HashMap<String, ContentAddress>>(&content)
            {
                for (hash, address) in index_data {
                    self.index.insert(hash, address);
                }
            }
        }

        // Load package index
        if package_index_path.exists() {
            let content = fs::read_to_string(&package_index_path).await?;
            if let Ok(package_data) =
                serde_json::from_str::<HashMap<String, PackageMetadata>>(&content)
            {
                for (key, metadata) in package_data {
                    self.package_index.insert(key, metadata);
                }
            }
        }

        Ok(())
    }

    async fn save_index(&self) -> Result<()> {
        let index_path = self.store_path.join("index").join("content.json");
        let package_index_path = self.store_path.join("index").join("packages.json");

        // Save content index
        let content_index: HashMap<String, ContentAddress> = self
            .index
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        let content_json = serde_json::to_string_pretty(&content_index)?;
        fs::write(&index_path, content_json).await?;

        // Save package index
        let package_index: HashMap<String, PackageMetadata> = self
            .package_index
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        let package_json = serde_json::to_string_pretty(&package_index)?;
        fs::write(&package_index_path, package_json).await?;

        Ok(())
    }

    pub fn format_size(bytes: u64) -> String {
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
}

#[derive(Debug)]
pub struct StoreStats {
    pub total_packages: u32,
    pub unique_content_count: u32,
    pub total_content_size: u64,
    pub duplicate_packages: u32,
    pub space_saved: u64,
}

impl Default for ContentStore {
    fn default() -> Self {
        Self::new()
    }
}

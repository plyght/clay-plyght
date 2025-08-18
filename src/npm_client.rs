use anyhow::{Result, anyhow};
use reqwest::Client;
use sha1::{Digest, Sha1};
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::package_info::{NpmRegistryResponse, PackageInfo};

#[derive(Clone)]
pub struct NpmClient {
    pub client: Client,
    registry_url: String,
}

impl NpmClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            registry_url: "https://registry.npmjs.org".to_string(),
        }
    }

    /// Fetch package information from NPM registry
    pub async fn get_package_info(&self, package_name: &str) -> Result<NpmRegistryResponse> {
        let url = format!("{}/{}", self.registry_url, package_name);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.npm.install-v1+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch package info: HTTP {}",
                response.status()
            ));
        }

        let package_info: NpmRegistryResponse = response.json().await?;
        Ok(package_info)
    }

    /// Download package tarball to specified path
    pub async fn download_package(
        &self,
        package_info: &PackageInfo,
        dest_path: &Path,
    ) -> Result<()> {
        let response = self.client.get(&package_info.dist.tarball).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to download package: HTTP {}",
                response.status()
            ));
        }

        // Ensure the parent directory exists
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Download and save the tarball
        let bytes = response.bytes().await?;

        // Verify integrity
        if !self.verify_package_integrity(&bytes, &package_info.dist.shasum)? {
            return Err(anyhow!(
                "Package integrity verification failed for {}. Expected: {}, Got different hash",
                package_info.name,
                package_info.dist.shasum
            ));
        }

        let mut file = fs::File::create(dest_path).await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        Ok(())
    }

    /// Verify package integrity using shasum
    pub fn verify_package_integrity(
        &self,
        file_data: &[u8],
        expected_shasum: &str,
    ) -> Result<bool> {
        // Compute SHA1 hash of the downloaded data
        let mut hasher = Sha1::new();
        hasher.update(file_data);
        let computed_hash = hasher.finalize();
        let computed_hash_hex = format!("{:x}", computed_hash);

        // Compare with expected hash
        let matches = computed_hash_hex == expected_shasum;

        Ok(matches)
    }
}

impl Default for NpmClient {
    fn default() -> Self {
        Self::new()
    }
}

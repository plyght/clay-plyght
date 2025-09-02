use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub main: Option<String>,
    pub bin: Option<Value>,
    pub dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "peerDependencies")]
    pub peer_dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "optionalDependencies")]
    pub optional_dependencies: Option<HashMap<String, String>>,
    pub dist: DistInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DistInfo {
    pub tarball: String,
    pub shasum: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NpmRegistryResponse {
    pub versions: HashMap<String, PackageInfo>,
    #[serde(rename = "dist-tags")]
    pub dist_tags: HashMap<String, String>,
}

impl NpmRegistryResponse {
    pub fn get_version(&self, version: &str) -> Option<&PackageInfo> {
        if version == "latest" {
            let latest_version = self.dist_tags.get("latest")?;
            self.versions.get(latest_version)
        } else {
            self.versions.get(version)
        }
    }

    pub fn get_latest_version(&self) -> Option<&PackageInfo> {
        let latest_version = self.dist_tags.get("latest")?;
        self.versions.get(latest_version)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub main: Option<String>,
    pub bin: Option<Value>,
    pub dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "devDependencies")]
    pub dev_dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "peerDependencies")]
    pub peer_dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "optionalDependencies")]
    pub optional_dependencies: Option<HashMap<String, String>>,
}

impl PackageJson {
    pub fn new() -> Self {
        Self {
            name: Some("my-project".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            main: Some("index.js".to_string()),
            bin: None,
            dependencies: Some(HashMap::new()),
            dev_dependencies: Some(HashMap::new()),
            peer_dependencies: Some(HashMap::new()),
            optional_dependencies: Some(HashMap::new()),
        }
    }

    pub fn add_dependency(&mut self, name: &str, version: &str) {
        if let Some(ref mut deps) = self.dependencies {
            deps.insert(name.to_string(), version.to_string());
        } else {
            let mut deps = HashMap::new();
            deps.insert(name.to_string(), version.to_string());
            self.dependencies = Some(deps);
        }
    }

    pub fn add_dev_dependency(&mut self, name: &str, version: &str) {
        if let Some(ref mut deps) = self.dev_dependencies {
            deps.insert(name.to_string(), version.to_string());
        } else {
            let mut deps = HashMap::new();
            deps.insert(name.to_string(), version.to_string());
            self.dev_dependencies = Some(deps);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub version: String,
    pub packages: HashMap<String, LockedPackage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LockedPackage {
    pub version: String,
    pub resolved: String,
    pub integrity: String,
    pub dependencies: Option<HashMap<String, String>>,
    pub required_by: Vec<String>, // Which packages depend on this one
}

impl LockFile {
    pub fn new() -> Self {
        Self {
            version: "1.0.0".to_string(),
            packages: HashMap::new(),
        }
    }

    pub fn add_package(
        &mut self,
        name: &str,
        version: &str,
        resolved: &str,
        integrity: &str,
        dependencies: Option<HashMap<String, String>>,
        required_by: &str,
    ) {
        let package = self
            .packages
            .entry(name.to_string())
            .or_insert(LockedPackage {
                version: version.to_string(),
                resolved: resolved.to_string(),
                integrity: integrity.to_string(),
                dependencies,
                required_by: Vec::new(),
            });

        // Add to required_by if not already present
        if !package.required_by.contains(&required_by.to_string()) {
            package.required_by.push(required_by.to_string());
        }
    }

    pub fn remove_package(&mut self, name: &str, required_by: &str) -> bool {
        if let Some(package) = self.packages.get_mut(name) {
            package.required_by.retain(|dep| dep != required_by);

            // If no packages depend on it, remove it completely
            if package.required_by.is_empty() {
                self.packages.remove(name);
                return true;
            }
        }
        false
    }

    pub fn can_remove_package(&self, name: &str, required_by: &str) -> (bool, Vec<String>) {
        if let Some(package) = self.packages.get(name) {
            let remaining_deps: Vec<String> = package
                .required_by
                .iter()
                .filter(|&dep| dep != required_by)
                .cloned()
                .collect();

            (remaining_deps.is_empty(), remaining_deps)
        } else {
            (true, Vec::new())
        }
    }
}

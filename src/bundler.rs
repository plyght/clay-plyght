use anyhow::{Result, anyhow};
use console::style;

use crate::cli_style::CliStyle;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};

pub struct Bundler {
    entry_points: Vec<PathBuf>,
    output_dir: PathBuf,
    resolve_cache: HashMap<String, PathBuf>,
    module_cache: HashMap<PathBuf, ModuleInfo>,
}

#[derive(Debug, Clone)]
struct ModuleInfo {
    content: String,
    dependencies: Vec<String>,
}

impl Bundler {
    pub fn new() -> Self {
        Self {
            entry_points: vec![PathBuf::from("src/index.js")],
            output_dir: PathBuf::from("dist"),
            resolve_cache: HashMap::new(),
            module_cache: HashMap::new(),
        }
    }

    pub async fn bundle(&mut self, output: Option<&str>, minify: bool, watch: bool) -> Result<()> {
        let output_path = output
            .map(PathBuf::from)
            .unwrap_or_else(|| self.output_dir.join("bundle.js"));

        if watch {
            println!("{}", CliStyle::info("Starting bundler in watch mode..."));
            self.bundle_with_watch(&output_path, minify).await
        } else {
            self.bundle_once(&output_path, minify).await
        }
    }

    async fn bundle_once(&mut self, output_path: &Path, minify: bool) -> Result<()> {
        let start_time = Instant::now();

        let bundle_spinner = CliStyle::create_spinner("Bundling application...");

        // Discover entry points
        bundle_spinner.set_message("Discovering entry points...");
        self.discover_entry_points().await?;

        if self.entry_points.is_empty() {
            bundle_spinner.finish_with_message(CliStyle::error("No entry points found"));
            return Err(anyhow!(
                "No entry points found. Expected src/index.js or main field in package.json"
            ));
        }

        // Build dependency graph
        bundle_spinner.set_message("Building dependency graph...");
        let mut bundled_modules = HashSet::new();
        let mut bundle_content = String::new();

        // Add runtime helpers
        bundle_content.push_str(&self.get_runtime_helpers());

        for entry_point in &self.entry_points.clone() {
            bundle_spinner.set_message(format!("Processing {}", entry_point.display()));
            self.resolve_and_bundle_module(entry_point, &mut bundle_content, &mut bundled_modules)
                .await?;
        }

        // Apply transformations
        if minify {
            bundle_spinner.set_message("Minifying bundle...");
            bundle_content = self.minify_bundle(&bundle_content).await?;
        }

        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Write bundle
        bundle_spinner.set_message("Writing bundle...");
        let mut file = fs::File::create(output_path).await?;
        let mut writer = BufWriter::new(&mut file);
        writer.write_all(bundle_content.as_bytes()).await?;
        writer.flush().await?;

        let duration = start_time.elapsed();
        let bundle_size = bundle_content.len();

        bundle_spinner.finish_with_message(format!(
            "Bundle created: {} ({}) in {}",
            style(output_path.display()).white().bold(),
            style(Self::format_size(bundle_size)).dim(),
            CliStyle::format_duration(duration)
        ));

        Ok(())
    }

    async fn bundle_with_watch(&mut self, output_path: &Path, minify: bool) -> Result<()> {
        use std::collections::HashSet;
        use tokio::time::{Duration, sleep};

        println!("{}", CliStyle::info("Performing initial bundle..."));
        self.bundle_once(output_path, minify).await?;

        let mut watched_files = HashSet::new();
        self.collect_watched_files(&mut watched_files).await?;

        println!(
            "{} Watching {} files for changes...",
            CliStyle::cyan_text(""),
            watched_files.len()
        );

        loop {
            sleep(Duration::from_millis(500)).await;

            let mut has_changes = false;
            let mut new_watched_files = HashSet::new();

            for file_path in &watched_files {
                if let Ok(metadata) = fs::metadata(file_path).await {
                    if metadata.modified().is_ok() {
                        // Simple change detection - in production, we'd use proper file watching
                        if !self.module_cache.contains_key(file_path) {
                            has_changes = true;
                            break;
                        }
                    }
                }
            }

            if has_changes {
                println!("{}", CliStyle::info("Changes detected, rebuilding..."));
                self.module_cache.clear();
                self.resolve_cache.clear();

                match self.bundle_once(output_path, minify).await {
                    Ok(()) => {
                        self.collect_watched_files(&mut new_watched_files).await?;
                        watched_files = new_watched_files;
                        println!("{}", CliStyle::success("Bundle updated successfully"));
                    }
                    Err(e) => {
                        println!("{}", CliStyle::error(&format!("Bundle error: {e}")));
                    }
                }
            }
        }
    }

    async fn discover_entry_points(&mut self) -> Result<()> {
        // Check package.json for main field
        if let Ok(content) = fs::read_to_string("package.json").await {
            if let Ok(package_json) = serde_json::from_str::<Value>(&content) {
                if let Some(main) = package_json.get("main").and_then(|m| m.as_str()) {
                    let main_path = PathBuf::from(main);
                    if main_path.exists() {
                        self.entry_points = vec![main_path];
                        return Ok(());
                    }
                }
            }
        }

        // Default entry points
        let candidates = vec![
            "src/index.js",
            "src/index.ts",
            "src/main.js",
            "src/main.ts",
            "index.js",
            "index.ts",
        ];

        self.entry_points.clear();
        for candidate in candidates {
            let path = PathBuf::from(candidate);
            if path.exists() {
                self.entry_points.push(path);
                break;
            }
        }

        Ok(())
    }

    async fn resolve_and_bundle_module(
        &mut self,
        module_path: &Path,
        bundle: &mut String,
        bundled: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        let canonical_path = fs::canonicalize(module_path)
            .await
            .unwrap_or_else(|_| module_path.to_path_buf());

        if bundled.contains(&canonical_path) {
            return Ok(());
        }

        let module_info = self.analyze_module(module_path).await?;
        bundled.insert(canonical_path.clone());

        // Bundle dependencies first
        for dep in &module_info.dependencies {
            if let Ok(dep_path) = self.resolve_module_path(dep, module_path).await {
                Box::pin(self.resolve_and_bundle_module(&dep_path, bundle, bundled)).await?;
            }
        }

        // Add this module to bundle
        bundle.push_str(&format!("\n// Module: {}\n", module_path.display()));
        bundle.push_str(&self.wrap_module(&module_info, &canonical_path)?);
        bundle.push('\n');

        Ok(())
    }

    async fn analyze_module(&mut self, module_path: &Path) -> Result<ModuleInfo> {
        if let Some(cached) = self.module_cache.get(module_path) {
            return Ok(cached.clone());
        }

        let content = fs::read_to_string(module_path).await?;
        let transformed_content = self.transform_module(&content, module_path).await?;

        let dependencies = self.extract_dependencies(&content)?;

        let module_info = ModuleInfo {
            content: transformed_content,
            dependencies,
        };

        self.module_cache
            .insert(module_path.to_path_buf(), module_info.clone());
        Ok(module_info)
    }

    async fn transform_module(&self, content: &str, module_path: &Path) -> Result<String> {
        let mut transformed = content.to_string();

        // TypeScript transpilation (basic)
        if module_path.extension().and_then(|s| s.to_str()) == Some("ts") {
            transformed = self.transpile_typescript(&transformed)?;
        }

        // Transform import/export statements to CommonJS-style for bundling
        transformed = self.transform_es_modules(&transformed)?;

        Ok(transformed)
    }

    fn transpile_typescript(&self, content: &str) -> Result<String> {
        // Basic TypeScript to JavaScript transpilation
        let mut result = content.to_string();

        // Remove type annotations (very basic implementation)
        result = regex::Regex::new(r":\s*[a-zA-Z_$][a-zA-Z0-9_$]*(<[^>]*>)?")
            .unwrap()
            .replace_all(&result, "")
            .to_string();

        // Remove interface declarations
        result = regex::Regex::new(r"interface\s+[^{]+\{[^}]*\}")
            .unwrap()
            .replace_all(&result, "")
            .to_string();

        // Remove type imports
        result = regex::Regex::new(r"import\s+type\s+[^;]+;")
            .unwrap()
            .replace_all(&result, "")
            .to_string();

        Ok(result)
    }

    fn transform_es_modules(&self, content: &str) -> Result<String> {
        let mut result = content.to_string();

        // Transform import statements
        let import_regex = regex::Regex::new(
            r#"import\s+(?:(?:\{([^}]+)\})|(?:(\w+)))\s+from\s+['"]([^'"]+)['"]"#,
        )?;
        result = import_regex
            .replace_all(&result, |caps: &regex::Captures| {
                let module_path = &caps[3];
                if let Some(named_imports) = caps.get(1) {
                    format!(
                        "const {{ {} }} = require('{}');",
                        named_imports.as_str(),
                        module_path
                    )
                } else if let Some(default_import) = caps.get(2) {
                    format!(
                        "const {} = require('{}');",
                        default_import.as_str(),
                        module_path
                    )
                } else {
                    format!("require('{module_path}');")
                }
            })
            .to_string();

        // Transform export statements
        let export_regex =
            regex::Regex::new(r"export\s+(?:default\s+)?(?:const|let|var|function|class)\s+(\w+)")?;
        result = export_regex
            .replace_all(&result, |caps: &regex::Captures| {
                let export_name = &caps[1];
                format!("const {export_name} = ")
            })
            .to_string();

        result.push_str("\nmodule.exports = { ");
        // Add exports (this is simplified)
        result.push_str(" };");

        Ok(result)
    }

    fn extract_dependencies(&self, content: &str) -> Result<Vec<String>> {
        let mut dependencies = Vec::new();

        // Extract from import statements
        let import_regex =
            regex::Regex::new(r#"(?:import\s+[^'"]*from\s+|require\s*\(\s*)['"]([^'"]+)['"]"#)?;

        for cap in import_regex.captures_iter(content) {
            if let Some(dep) = cap.get(1) {
                let dep_str = dep.as_str();
                if !dep_str.starts_with('.') && !dep_str.starts_with('/') {
                    // This is a node_modules dependency
                    dependencies.push(dep_str.to_string());
                } else {
                    // This is a relative import
                    dependencies.push(dep_str.to_string());
                }
            }
        }

        Ok(dependencies)
    }

    async fn resolve_module_path(
        &mut self,
        module_spec: &str,
        from_path: &Path,
    ) -> Result<PathBuf> {
        let cache_key = format!("{}:{}", from_path.display(), module_spec);

        if let Some(cached) = self.resolve_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let resolved = if module_spec.starts_with('.') {
            // Relative import
            let from_dir = from_path.parent().unwrap_or(Path::new("."));
            let candidate = from_dir.join(module_spec);

            self.resolve_file_extensions(&candidate).await?
        } else {
            // Node modules import
            self.resolve_node_modules(module_spec, from_path).await?
        };

        self.resolve_cache.insert(cache_key, resolved.clone());
        Ok(resolved)
    }

    async fn resolve_file_extensions(&self, base_path: &Path) -> Result<PathBuf> {
        let extensions = vec!["", ".js", ".ts", ".json"];

        for ext in extensions {
            let candidate = if ext.is_empty() {
                base_path.to_path_buf()
            } else {
                PathBuf::from(format!("{}{}", base_path.display(), ext))
            };

            if candidate.exists() {
                return Ok(candidate);
            }

            // Try index file in directory
            if candidate.is_dir() {
                let index_candidate = candidate.join(format!("index{ext}"));
                if index_candidate.exists() {
                    return Ok(index_candidate);
                }
            }
        }

        Err(anyhow!("Could not resolve module: {}", base_path.display()))
    }

    async fn resolve_node_modules(&self, module_name: &str, from_path: &Path) -> Result<PathBuf> {
        let mut current_dir = from_path.parent().unwrap_or(Path::new("."));

        loop {
            let node_modules = current_dir.join("node_modules").join(module_name);

            if node_modules.exists() {
                // Check for package.json main field
                let package_json_path = node_modules.join("package.json");
                if package_json_path.exists() {
                    if let Ok(content) = fs::read_to_string(&package_json_path).await {
                        if let Ok(package_json) = serde_json::from_str::<Value>(&content) {
                            if let Some(main) = package_json.get("main").and_then(|m| m.as_str()) {
                                let main_path = node_modules.join(main);
                                if main_path.exists() {
                                    return Ok(main_path);
                                }
                            }
                        }
                    }
                }

                // Try index files
                let extensions = vec!["index.js", "index.ts"];
                for ext in extensions {
                    let index_path = node_modules.join(ext);
                    if index_path.exists() {
                        return Ok(index_path);
                    }
                }

                return Ok(node_modules);
            }

            match current_dir.parent() {
                Some(parent) => current_dir = parent,
                None => break,
            }
        }

        Err(anyhow!("Could not resolve node module: {}", module_name))
    }

    fn wrap_module(&self, module_info: &ModuleInfo, module_path: &Path) -> Result<String> {
        let wrapped = format!(
            r#"
// Module: {}
(function(module, exports, require) {{
{}
}}).call(this, 
  {{ exports: {{}} }}, 
  {{}}, 
  function(id) {{ return __clay_require(id, "{}"); }}
);
"#,
            module_path.display(),
            module_info.content,
            module_path.display()
        );

        Ok(wrapped)
    }

    fn get_runtime_helpers(&self) -> String {
        r#"
// Clay bundler runtime
(function() {
  var __clay_modules = {};
  var __clay_cache = {};
  
  function __clay_require(id, from) {
    if (__clay_cache[id]) {
      return __clay_cache[id].exports;
    }
    
    var module = { exports: {} };
    __clay_cache[id] = module;
    
    if (__clay_modules[id]) {
      __clay_modules[id].call(module.exports, module, module.exports, __clay_require);
    }
    
    return module.exports;
  }
  
  window.__clay_require = __clay_require;
  window.__clay_modules = __clay_modules;
})();
"#
        .to_string()
    }

    async fn minify_bundle(&self, content: &str) -> Result<String> {
        // Basic minification
        let mut minified = content.to_string();

        // Remove comments
        minified = regex::Regex::new(r"//[^\n]*\n")
            .unwrap()
            .replace_all(&minified, "\n")
            .to_string();

        minified = regex::Regex::new(r"/\*[\s\S]*?\*/")
            .unwrap()
            .replace_all(&minified, "")
            .to_string();

        // Remove extra whitespace
        minified = regex::Regex::new(r"\s+")
            .unwrap()
            .replace_all(&minified, " ")
            .to_string();

        // Remove unnecessary semicolons and spaces
        minified = minified.replace("; ", ";");
        minified = minified.replace(" {", "{");
        minified = minified.replace("} ", "}");

        Ok(minified)
    }

    async fn collect_watched_files(&self, files: &mut HashSet<PathBuf>) -> Result<()> {
        for path in self.module_cache.keys() {
            files.insert(path.clone());
        }
        Ok(())
    }

    fn format_size(bytes: usize) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size as usize, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }
}

impl Default for Bundler {
    fn default() -> Self {
        Self::new()
    }
}

use anyhow::Result;
use clap::{Parser, Subcommand};

mod npm_client;
mod package_info;
mod package_manager;

use package_manager::PackageManager;

#[derive(Parser)]
#[command(name = "clay")]
#[command(about = "Clay - A fast, modern Node.js package manager built in Rust")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install packages with parallel downloads
    #[command(
        alias = "i",
        alias = "add",
        long_about = "Install packages with integrity verification and parallel dependency resolution.\nIf no package is specified, installs all dependencies from package.json.\nUse package@version syntax to specify versions (e.g. lodash@4.17.21)."
    )]
    Install {
        /// Package names to install (use package@version for specific versions)
        packages: Vec<String>,
        /// Install as development dependencies
        #[arg(long)]
        dev: bool,
        /// Use JSON format for lock file instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Remove packages and clean up dependencies
    Uninstall {
        /// Package names to remove
        packages: Vec<String>,
    },
    /// Show installed packages
    List,
    /// Manage package cache
    #[command(subcommand)]
    Cache(CacheCommands),
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Show cache information
    Info,
    /// Clear all cached packages
    Clear,
    /// Show cache directory path
    Dir,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install {
            packages,
            dev,
            json,
        } => {
            let package_manager = PackageManager::with_toml_lock(!json);

            let package_specs = if packages.is_empty() {
                // Read dependencies from package.json
                package_manager.get_package_json_dependencies(dev).await?
            } else {
                // Parse package specifications
                let mut specs = Vec::new();
                for package_spec in packages {
                    let (package_name, version) = if let Some(at_pos) = package_spec.rfind('@') {
                        if at_pos > 0 {
                            // Split at the last @ symbol
                            let name = &package_spec[..at_pos];
                            let version = &package_spec[at_pos + 1..];
                            (name.to_string(), version.to_string())
                        } else {
                            // @ at the beginning, treat as package name
                            (package_spec, "latest".to_string())
                        }
                    } else {
                        // No @ symbol, use latest version
                        (package_spec, "latest".to_string())
                    };
                    specs.push((package_name, version));
                }
                specs
            };

            // Always use unified interface with resolver
            package_manager
                .install_multiple_packages(package_specs, dev)
                .await?;
        }
        Commands::Uninstall { packages } => {
            let package_manager = PackageManager::new();
            for package_name in packages {
                package_manager.uninstall_package(&package_name).await?;
            }
        }
        Commands::List => {
            let package_manager = PackageManager::new();
            package_manager.list_installed_packages().await?;
        }
        Commands::Cache(cache_cmd) => {
            let package_manager = PackageManager::new();
            match cache_cmd {
                CacheCommands::Info => {
                    package_manager.cache_info().await?;
                }
                CacheCommands::Clear => {
                    package_manager.cache_clear().await?;
                }
                CacheCommands::Dir => {
                    package_manager.cache_dir().await?;
                }
            }
        }
    }

    Ok(())
}

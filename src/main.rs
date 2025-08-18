use anyhow::Result;
use clap::{Parser, Subcommand};

mod npm_client;
mod package_info;
mod package_manager;

use package_manager::PackageManager;

#[derive(Parser)]
#[command(name = "fnpm")]
#[command(about = "A fast, modern Node.js package manager")]
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
    },
    /// Remove packages and clean up dependencies
    Uninstall {
        /// Package names to remove
        packages: Vec<String>,
    },
    /// Show installed packages
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let package_manager = PackageManager::new();

    match cli.command {
        Commands::Install { packages, dev } => {
            if packages.is_empty() {
                // Install dependencies from package.json
                package_manager.install_dependencies().await?;
            } else {
                // Parse package specifications
                let mut package_specs = Vec::new();
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
                    package_specs.push((package_name, version));
                }

                // Install all packages with unified interface
                package_manager
                    .install_multiple_packages(package_specs, dev)
                    .await?;
            }
        }
        Commands::Uninstall { packages } => {
            for package_name in packages {
                package_manager.uninstall_package(&package_name).await?;
            }
        }
        Commands::List => {
            package_manager.list_installed_packages().await?;
        }
    }

    Ok(())
}

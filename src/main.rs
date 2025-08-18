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
        long_about = "Install a package with integrity verification and parallel dependency resolution.\nIf no package is specified, installs all dependencies from package.json."
    )]
    Install {
        /// Package name to install
        package: Option<String>,
        /// Specific version to install
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Remove packages and clean up dependencies
    Uninstall {
        /// Package name to remove
        package: String,
    },
    /// Show installed packages
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let package_manager = PackageManager::new();

    match cli.command {
        Commands::Install { package, version } => {
            if let Some(package_name) = package {
                // Install specific package
                let version_str = version.unwrap_or_else(|| "latest".to_string());
                package_manager
                    .install_package(&package_name, &version_str)
                    .await?;
            } else {
                // Install dependencies from package.json
                package_manager.install_dependencies().await?;
            }
        }
        Commands::Uninstall { package } => {
            package_manager.uninstall_package(&package).await?;
        }
        Commands::List => {
            package_manager.list_installed_packages().await?;
        }
    }

    Ok(())
}

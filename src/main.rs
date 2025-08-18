use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::Command;

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
    #[command(alias = "i", alias = "add")]
    Install {
        packages: Vec<String>,

        #[arg(long)]
        dev: bool,

        #[arg(long)]
        json: bool,
    },

    Uninstall {
        packages: Vec<String>,
    },

    List,

    Upgrade {
        #[arg(long, short)]
        yes: bool,
    },

    #[command(subcommand)]
    Cache(CacheCommands),
}

#[derive(Subcommand)]
enum CacheCommands {
    Info,

    Clear,

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
                package_manager.get_package_json_dependencies(dev).await?
            } else {
                let mut specs = Vec::new();
                for package_spec in packages {
                    let (package_name, version) = if let Some(at_pos) = package_spec.rfind('@') {
                        if at_pos > 0 {
                            let name = &package_spec[..at_pos];
                            let version = &package_spec[at_pos + 1..];
                            (name.to_string(), version.to_string())
                        } else {
                            (package_spec, "latest".to_string())
                        }
                    } else {
                        (package_spec, "latest".to_string())
                    };
                    specs.push((package_name, version));
                }
                specs
            };

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
        Commands::Upgrade { yes } => {
            upgrade_clay(yes).await?;
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

async fn upgrade_clay(skip_confirmation: bool) -> Result<()> {
    use console::style;
    use std::io::{self, Write};

    println!("{}", style("🚀 Clay Upgrade").bold().blue());
    println!("This will download and run the latest Clay installer.");
    println!();

    if !skip_confirmation {
        print!("Do you want to continue? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("Upgrade cancelled.");
            return Ok(());
        }
    }

    println!("{}", style("Downloading installer...").cyan());

    let install_script_url =
        "https://raw.githubusercontent.com/lassejlv/clay/main/scripts/install.sh";
    let response = reqwest::get(install_script_url).await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download installer: HTTP {}", response.status());
    }

    let script_content = response.text().await?;

    let temp_dir = std::env::temp_dir();
    let script_path = temp_dir.join("clay_install.sh");
    std::fs::write(&script_path, script_content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms)?;
    }

    println!("{}", style("Running installer...").cyan());
    println!();

    let status = Command::new("bash").arg(&script_path).status()?;

    let _ = std::fs::remove_file(&script_path);

    if status.success() {
        println!();
        println!(
            "{}",
            style("✅ Upgrade completed successfully!").green().bold()
        );
        println!(
            "Please restart your shell or run 'source ~/.bashrc' to ensure the new version is loaded."
        );
    } else {
        anyhow::bail!(
            "Installer failed with exit code: {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

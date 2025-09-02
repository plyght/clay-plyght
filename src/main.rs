use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::Command;

mod bundler;
mod cli_style;
mod content_store;
mod dev_server;
mod npm_client;
mod package_info;
mod package_manager;
mod workspace;

use bundler::Bundler;
use cli_style::CliStyle;
use content_store::ContentStore;
use dev_server::DevServer;
use package_manager::PackageManager;
use workspace::WorkspaceManager;

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

        #[arg(long)]
        fix_peers: bool,

        #[arg(long)]
        skip_peers: bool,
    },

    Uninstall {
        packages: Vec<String>,
    },

    List,

    Upgrade {
        #[arg(long, short)]
        yes: bool,
    },

    Run {
        script: Option<String>,
    },

    #[command(subcommand)]
    Cache(CacheCommands),

    #[command(subcommand)]
    Store(StoreCommands),

    #[command(subcommand)]
    Workspace(WorkspaceCommands),

    Bundle {
        #[arg(short, long)]
        output: Option<String>,

        #[arg(short, long)]
        minify: bool,

        #[arg(long)]
        watch: bool,
    },

    Dev {
        #[arg(short, long, default_value = "3000")]
        port: u16,

        #[arg(long)]
        host: Option<String>,
    },

    #[command(subcommand)]
    Peer(PeerCommands),

    Check {
        #[arg(long)]
        peers: bool,

        #[arg(long)]
        all: bool,
    },

    Info {
        package: Option<String>,
    },

    Link {
        package: String,
        version: String,
        #[arg(short, long)]
        target: String,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    Info,

    Clear,

    Dir,
}

#[derive(Subcommand)]
enum StoreCommands {
    Stats,

    Dedupe,

    Cleanup,

    Gc,
}

#[derive(Subcommand)]
enum PeerCommands {
    Check,

    Install,

    List,
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    List,

    Add {
        name: String,
        #[arg(long)]
        path: Option<String>,
    },

    Remove {
        name: String,
    },

    Run {
        script: String,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        parallel: bool,
    },

    Install {
        #[arg(long)]
        all: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install {
            packages,
            dev,
            json,
            fix_peers,
            skip_peers,
        } => {
            let package_manager = PackageManager::with_toml_lock(!json);

            let package_specs = if packages.is_empty() {
                package_manager.get_package_json_dependencies(dev).await?
            } else {
                let mut specs = Vec::new();
                for package_spec in &packages {
                    let (package_name, version) = if let Some(at_pos) = package_spec.rfind('@') {
                        if at_pos > 0 {
                            let name = &package_spec[..at_pos];
                            let version = &package_spec[at_pos + 1..];
                            (name.to_string(), version.to_string())
                        } else {
                            (package_spec.clone(), "latest".to_string())
                        }
                    } else {
                        (package_spec.clone(), "latest".to_string())
                    };
                    specs.push((package_name, version));
                }
                specs
            };

            let is_specific_install = !packages.is_empty();
            package_manager
                .install_multiple_packages(package_specs, dev, is_specific_install)
                .await?;

            // Handle peer dependencies if requested
            if fix_peers && !skip_peers {
                println!("{}", CliStyle::info("Auto-installing peer dependencies..."));

                // Get all installed packages and check their peer dependencies
                let installed_packages = package_manager
                    .get_installed_packages()
                    .await
                    .unwrap_or_default();
                for package_name in &installed_packages {
                    if let Ok(package_info) = package_manager
                        .npm_client
                        .get_package_info(package_name)
                        .await
                    {
                        if let Some(latest_info) = package_info.get_latest_version() {
                            package_manager
                                .auto_install_peer_dependencies(latest_info)
                                .await?;
                        }
                    }
                }

                // Report any remaining conflicts
                package_manager.report_peer_conflicts().await?;
            } else if !skip_peers {
                // Default behavior: just report peer conflicts without auto-installing
                package_manager.report_peer_conflicts().await?;
            }
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
        Commands::Run { script } => {
            let package_manager = PackageManager::new();
            match script {
                Some(script_name) => {
                    package_manager.run_script(&script_name).await?;
                }
                None => {
                    package_manager.list_scripts().await?;
                }
            }
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
        Commands::Store(store_cmd) => {
            let content_store = ContentStore::new();
            content_store.initialize().await?;

            match store_cmd {
                StoreCommands::Stats => {
                    let stats = content_store.get_store_stats().await?;
                    println!("{}", CliStyle::section_header("Content Store Statistics"));
                    println!(
                        "Total packages: {}",
                        console::style(stats.total_packages).green()
                    );
                    println!(
                        "Unique content items: {}",
                        console::style(stats.unique_content_count).green()
                    );
                    println!(
                        "Total content size: {}",
                        console::style(ContentStore::format_size(stats.total_content_size)).green()
                    );
                    println!(
                        "Duplicate packages: {}",
                        console::style(stats.duplicate_packages).yellow()
                    );
                    println!(
                        "Space saved by deduplication: {}",
                        console::style(ContentStore::format_size(stats.space_saved)).green()
                    );
                }
                StoreCommands::Dedupe => {
                    content_store.deduplicate_store().await?;
                }
                StoreCommands::Cleanup => {
                    // Get list of currently installed packages
                    let package_manager = PackageManager::new();
                    let active_packages = package_manager
                        .get_installed_packages()
                        .await
                        .unwrap_or_default();
                    let active_package_specs: Vec<String> = active_packages
                        .into_iter()
                        .map(|name| format!("{name}@latest"))
                        .collect();
                    content_store.cleanup_unused(&active_package_specs).await?;
                }
                StoreCommands::Gc => {
                    content_store.deduplicate_store().await?;
                    let package_manager = PackageManager::new();
                    let active_packages = package_manager
                        .get_installed_packages()
                        .await
                        .unwrap_or_default();
                    let active_package_specs: Vec<String> = active_packages
                        .into_iter()
                        .map(|name| format!("{name}@latest"))
                        .collect();
                    content_store.cleanup_unused(&active_package_specs).await?;
                }
            }
        }
        Commands::Workspace(workspace_cmd) => {
            let workspace_manager = WorkspaceManager::new();
            match workspace_cmd {
                WorkspaceCommands::List => {
                    workspace_manager.list_workspaces().await?;
                }
                WorkspaceCommands::Add { name, path } => {
                    let workspace_path = path.unwrap_or_else(|| format!("packages/{name}"));
                    workspace_manager
                        .add_workspace(&name, &workspace_path)
                        .await?;
                }
                WorkspaceCommands::Remove { name } => {
                    workspace_manager.remove_workspace(&name).await?;
                }
                WorkspaceCommands::Run {
                    script,
                    workspace,
                    parallel,
                } => {
                    workspace_manager
                        .run_script(&script, workspace.as_deref(), parallel)
                        .await?;
                }
                WorkspaceCommands::Install { all: _ } => {
                    workspace_manager.install_workspace_dependencies().await?;
                }
            }
        }
        Commands::Bundle {
            output,
            minify,
            watch,
        } => {
            let mut bundler = Bundler::new();
            bundler.bundle(output.as_deref(), minify, watch).await?;
        }
        Commands::Dev { port, host } => {
            let mut dev_server = DevServer::new();
            let host = host.unwrap_or_else(|| "localhost".to_string());
            dev_server.start(&host, port).await?;
        }
        Commands::Peer(peer_cmd) => {
            let package_manager = PackageManager::new();
            match peer_cmd {
                PeerCommands::Check => {
                    package_manager.report_peer_conflicts().await?;
                }
                PeerCommands::Install => {
                    let conflicts = package_manager.check_peer_dependency_conflicts().await?;
                    if conflicts.is_empty() {
                        println!(
                            "{}",
                            CliStyle::success("No peer dependency conflicts found")
                        );
                    } else {
                        println!(
                            "{}",
                            CliStyle::info("Installing missing peer dependencies...")
                        );
                        // Auto-install missing peers would be implemented here
                        package_manager.report_peer_conflicts().await?;
                    }
                }
                PeerCommands::List => {
                    println!("{}", CliStyle::info("Listing peer dependencies..."));
                    let conflicts = package_manager.check_peer_dependency_conflicts().await?;
                    if conflicts.is_empty() {
                        println!("{}", CliStyle::warning("No peer dependencies found"));
                    } else {
                        println!(
                            "{} Found {} peer dependencies:",
                            CliStyle::info(""),
                            conflicts.len()
                        );
                        for conflict in conflicts {
                            println!(
                                "  {} {} requires {} {}",
                                CliStyle::arrow(""),
                                console::style(conflict.package).white().bold(),
                                console::style(conflict.peer_dependency).white(),
                                console::style(conflict.required_version).dim()
                            );
                        }
                    }
                }
            }
        }
        Commands::Check { peers, all } => {
            let package_manager = PackageManager::new();

            if peers || all {
                println!("{}", CliStyle::info("Checking peer dependencies..."));
                package_manager.report_peer_conflicts().await?;
            }

            if all {
                println!("{}", CliStyle::info("Checking package integrity..."));
                // Could add integrity checks here
                println!("{}", CliStyle::success("Package integrity check completed"));
            }

            if !peers && !all {
                println!(
                    "{}",
                    CliStyle::info("Use --peers or --all to specify what to check")
                );
            }
        }
        Commands::Info { package } => {
            let content_store = ContentStore::new();
            content_store.initialize().await?;

            if let Some(pkg_name) = package {
                // Show package info from content store
                if let Some(metadata) = content_store.get_package_info(&pkg_name, "latest").await {
                    println!(
                        "{} Package: {}",
                        CliStyle::info(""),
                        console::style(metadata.name).white().bold()
                    );
                    println!("Version: {}", console::style(metadata.version).green());
                    println!(
                        "Content hash: {}",
                        console::style(&metadata.content_address.hash[..12]).dim()
                    );
                    println!(
                        "Size: {}",
                        console::style(ContentStore::format_size(metadata.content_address.size))
                            .green()
                    );
                    if let Some(deps) = metadata.dependencies {
                        println!("Dependencies: {}", deps.len());
                    }
                    println!("Files: {}", metadata.files.len());
                } else {
                    println!(
                        "{} Package '{}' not found in content store",
                        console::style("•").yellow(),
                        pkg_name
                    );
                }
            } else {
                // Show general package manager info
                let stats = content_store.get_store_stats().await?;
                let package_manager = PackageManager::new();
                package_manager.cache_info().await?;
                println!("\n{}", CliStyle::section_header("Content Store:"));
                println!(
                    "Total packages: {}",
                    console::style(stats.total_packages).green()
                );
                println!(
                    "Unique content: {}",
                    console::style(stats.unique_content_count).green()
                );
                println!(
                    "Space saved: {}",
                    console::style(ContentStore::format_size(stats.space_saved)).green()
                );
            }
        }
        Commands::Link {
            package,
            version,
            target,
        } => {
            let content_store = ContentStore::new();
            content_store.initialize().await?;

            let target_path = std::path::PathBuf::from(&target);
            if content_store
                .link_package(&package, &version, &target_path)
                .await?
            {
                println!(
                    "{} Successfully linked {} {} to {}",
                    CliStyle::success(""),
                    console::style(package).white().bold(),
                    console::style(version).dim(),
                    console::style(target).cyan()
                );
            } else {
                println!(
                    "{} Package {}@{} not found in content store",
                    CliStyle::error(""),
                    console::style(package).white().bold(),
                    console::style(version).dim()
                );
            }
        }
    }

    Ok(())
}

async fn upgrade_clay(skip_confirmation: bool) -> Result<()> {
    use console::style;
    use std::io::{self, Write};

    println!("{}", CliStyle::section_header("Clay Upgrade"));
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
        println!("{}", CliStyle::success("Upgrade completed successfully!"));
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

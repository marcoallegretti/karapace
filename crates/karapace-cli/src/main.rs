mod commands;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use commands::{EXIT_FAILURE, EXIT_MANIFEST_ERROR, EXIT_STORE_ERROR};
use karapace_core::{install_signal_handler, BuildOptions, Engine};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "karapace",
    version,
    about = "Deterministic environment engine for immutable systems"
)]
struct Cli {
    /// Path to the Karapace store directory.
    #[arg(long, default_value = "~/.local/share/karapace")]
    store: String,

    /// Output results as structured JSON.
    #[arg(long, default_value_t = false, global = true)]
    json: bool,

    /// Enable verbose (debug) logging output.
    #[arg(short, long, default_value_t = false, global = true)]
    verbose: bool,

    /// Enable trace-level logging (more detailed than --verbose).
    #[arg(long, default_value_t = false, global = true)]
    trace: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    New {
        name: String,
        #[arg(long)]
        template: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Build an environment from a manifest.
    Build {
        /// Path to manifest TOML file.
        #[arg(default_value = "karapace.toml")]
        manifest: PathBuf,
        /// Human-readable name for the environment.
        #[arg(long)]
        name: Option<String>,
        /// Require an existing lock file and fail if resolved state would drift.
        #[arg(long, default_value_t = false)]
        locked: bool,
        /// Forbid all network access (host downloads and container networking).
        #[arg(long, default_value_t = false)]
        offline: bool,
        /// Require base.image to be a pinned http(s) URL.
        #[arg(long, default_value_t = false)]
        require_pinned_image: bool,
    },
    /// Destroy and rebuild an environment from manifest.
    Rebuild {
        /// Path to manifest TOML file.
        #[arg(default_value = "karapace.toml")]
        manifest: PathBuf,
        /// Human-readable name for the environment.
        #[arg(long)]
        name: Option<String>,
        /// Require an existing lock file and fail if resolved state would drift.
        #[arg(long, default_value_t = false)]
        locked: bool,
        /// Forbid all network access (host downloads and container networking).
        #[arg(long, default_value_t = false)]
        offline: bool,
        /// Require base.image to be a pinned http(s) URL.
        #[arg(long, default_value_t = false)]
        require_pinned_image: bool,
    },

    /// Rewrite a manifest to use an explicit pinned base image reference.
    Pin {
        /// Path to manifest TOML file.
        #[arg(default_value = "karapace.toml")]
        manifest: PathBuf,
        /// Exit non-zero if the manifest is not already pinned.
        #[arg(long, default_value_t = false)]
        check: bool,
        /// After pinning, write/update karapace.lock by running a build.
        #[arg(long, default_value_t = false)]
        write_lock: bool,
    },
    /// Enter a built environment (use -- to pass a command instead of interactive shell).
    Enter {
        /// Environment ID (full or short).
        env_id: String,
        /// Command to run inside the environment (after --).
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Execute a command inside a built environment (non-interactive).
    Exec {
        /// Environment ID (full or short).
        env_id: String,
        /// Command and arguments to run.
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
    /// Destroy an environment and its overlay.
    Destroy {
        /// Environment ID.
        env_id: String,
    },
    /// Stop a running environment.
    Stop {
        /// Environment ID.
        env_id: String,
    },
    /// Freeze an environment (prevent further writes).
    Freeze {
        /// Environment ID.
        env_id: String,
    },
    /// Archive an environment (preserve but prevent entry).
    Archive {
        /// Environment ID.
        env_id: String,
    },
    /// List all known environments.
    List,
    /// Inspect environment metadata.
    Inspect {
        /// Environment ID.
        env_id: String,
    },
    /// Show drift in the writable overlay of an environment.
    Diff {
        /// Environment ID.
        env_id: String,
    },
    /// List snapshots for an environment.
    Snapshots {
        /// Environment ID.
        env_id: String,
    },
    /// Commit overlay drift into the content store as a snapshot.
    Commit {
        /// Environment ID.
        env_id: String,
    },
    /// Restore an environment's overlay from a snapshot.
    Restore {
        /// Environment ID.
        env_id: String,
        /// Snapshot layer hash to restore from.
        snapshot: String,
    },
    /// Run garbage collection on the store.
    Gc {
        /// Only report what would be removed.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Verify store integrity.
    VerifyStore,
    /// Push an environment to a remote store.
    Push {
        /// Environment ID, short ID, or name.
        env_id: String,
        /// Registry tag (e.g. "my-env@latest"). If omitted, pushed without a tag.
        #[arg(long)]
        tag: Option<String>,
        /// Remote store URL (overrides config file).
        #[arg(long)]
        remote: Option<String>,
    },
    /// Pull an environment from a remote store.
    Pull {
        /// Registry reference (e.g. "my-env@latest") or raw env_id.
        reference: String,
        /// Remote store URL (overrides config file).
        #[arg(long)]
        remote: Option<String>,
    },
    /// Rename an environment.
    Rename {
        /// Environment ID or current name.
        env_id: String,
        /// New name for the environment.
        new_name: String,
    },
    /// Generate shell completions for bash, zsh, fish, elvish, or powershell.
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
    /// Generate man pages in the specified directory.
    ManPages {
        /// Output directory for man pages.
        #[arg(default_value = "man")]
        dir: PathBuf,
    },
    /// Launch the terminal UI.
    Tui,
    /// Run diagnostic checks on the system and store.
    Doctor,
    /// Check store version and show migration guidance.
    Migrate,
}

#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe")
            || msg.contains("broken pipe")
            || msg.contains("os error 32")
            || msg.contains("failed printing to stdout")
        {
            std::process::exit(0);
        }
        default_hook(info);
    }));

    let cli = Cli::parse();

    let default_level = if cli.trace {
        "trace"
    } else if cli.verbose {
        "debug"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("KARAPACE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level)),
        )
        .with_target(false)
        .without_time()
        .init();

    install_signal_handler();

    let store_path = expand_tilde(&cli.store);
    let engine = Engine::new(&store_path);
    let json_output = cli.json;

    let needs_runtime = matches!(
        cli.command,
        Commands::Build { .. }
            | Commands::Enter { .. }
            | Commands::Exec { .. }
            | Commands::Rebuild { .. }
            | Commands::Pin {
                write_lock: true,
                ..
            }
            | Commands::Tui
    );
    if needs_runtime && std::env::var("KARAPACE_SKIP_PREREQS").as_deref() != Ok("1") {
        let missing = karapace_runtime::check_namespace_prereqs();
        if !missing.is_empty() {
            eprintln!("error: {}", karapace_runtime::format_missing(&missing));
            return ExitCode::from(EXIT_FAILURE);
        }
    }

    let result = match cli.command {
        Commands::New {
            name,
            template,
            force,
        } => commands::new::run(&name, template.as_deref(), force, json_output),
        Commands::Build {
            manifest,
            name,
            locked,
            offline,
            require_pinned_image,
        } => commands::build::run(
            &engine,
            &store_path,
            &manifest,
            name.as_deref(),
            BuildOptions {
                locked,
                offline,
                require_pinned_image,
            },
            json_output,
        ),
        Commands::Rebuild {
            manifest,
            name,
            locked,
            offline,
            require_pinned_image,
        } => commands::rebuild::run(
            &engine,
            &store_path,
            &manifest,
            name.as_deref(),
            BuildOptions {
                locked,
                offline,
                require_pinned_image,
            },
            json_output,
        ),
        Commands::Pin {
            manifest,
            check,
            write_lock,
        } => commands::pin::run(&manifest, check, write_lock, json_output, Some(&store_path)),
        Commands::Enter { env_id, command } => {
            commands::enter::run(&engine, &store_path, &env_id, &command)
        }
        Commands::Exec { env_id, command } => {
            commands::exec::run(&engine, &store_path, &env_id, &command, json_output)
        }
        Commands::Destroy { env_id } => commands::destroy::run(&engine, &store_path, &env_id),
        Commands::Stop { env_id } => commands::stop::run(&engine, &store_path, &env_id),
        Commands::Freeze { env_id } => commands::freeze::run(&engine, &store_path, &env_id),
        Commands::Archive { env_id } => commands::archive::run(&engine, &store_path, &env_id),
        Commands::List => commands::list::run(&engine, json_output),
        Commands::Inspect { env_id } => commands::inspect::run(&engine, &env_id, json_output),
        Commands::Diff { env_id } => commands::diff::run(&engine, &env_id, json_output),
        Commands::Snapshots { env_id } => {
            commands::snapshots::run(&engine, &store_path, &env_id, json_output)
        }
        Commands::Commit { env_id } => {
            commands::commit::run(&engine, &store_path, &env_id, json_output)
        }
        Commands::Restore { env_id, snapshot } => {
            commands::restore::run(&engine, &store_path, &env_id, &snapshot, json_output)
        }
        Commands::Gc { dry_run } => commands::gc::run(&engine, &store_path, dry_run, json_output),
        Commands::VerifyStore => commands::verify_store::run(&engine, json_output),
        Commands::Push {
            env_id,
            tag,
            remote,
        } => commands::push::run(
            &engine,
            &env_id,
            tag.as_deref(),
            remote.as_deref(),
            json_output,
        ),
        Commands::Pull { reference, remote } => {
            commands::pull::run(&engine, &reference, remote.as_deref(), json_output)
        }
        Commands::Rename { env_id, new_name } => {
            commands::rename::run(&engine, &store_path, &env_id, &new_name)
        }
        Commands::Completions { shell } => commands::completions::run::<Cli>(shell),
        Commands::ManPages { dir } => commands::man_pages::run::<Cli>(&dir),
        Commands::Tui => commands::tui::run(&store_path, json_output),
        Commands::Doctor => commands::doctor::run(&store_path, json_output),
        Commands::Migrate => commands::migrate::run(&store_path, json_output),
    };

    match result {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("error: {msg}");
            let code = if msg.starts_with("manifest error:")
                || msg.starts_with("failed to parse manifest")
                || msg.starts_with("failed to read manifest")
            {
                EXIT_MANIFEST_ERROR
            } else if msg.starts_with("store error:") || msg.starts_with("store lock:") {
                EXIT_STORE_ERROR
            } else {
                EXIT_FAILURE
            };
            ExitCode::from(code)
        }
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(path)
}

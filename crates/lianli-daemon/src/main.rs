mod controllers;
mod desktop_display;
mod ipc;
mod openrgb_server;
mod persistence;
mod pidlock;
mod pixel_cleaner;
mod service;
mod template_store;
mod thermal_alert;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

fn default_config_path(system: bool) -> PathBuf {
    if system {
        return PathBuf::from("/var/lib/lianli/config.json");
    }
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".config")
        });
    config_dir.join("lianli").join("config.json")
}

fn default_socket_path(system: bool) -> PathBuf {
    if system {
        PathBuf::from("/run/lianli/lianli-daemon.sock")
    } else {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(runtime_dir).join("lianli-daemon.sock")
    }
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Linux daemon for Lian Li fan control and LCD streaming"
)]
struct Cli {
    /// Path to the configuration file
    #[arg(long)]
    config: Option<PathBuf>,

    /// IPC socket path
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Run as a system service
    #[arg(long)]
    system: bool,

    /// Logging verbosity (error, warn, info, debug, trace)
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// LCD utilities and maintenance
    Lcd {
        #[command(subcommand)]
        command: LcdCommands,
    },
}

#[derive(Subcommand, Debug)]
enum LcdCommands {
    /// Run pixel conditioning / exercise loop to clear image retention
    Clean {
        /// Target device ID (or all detected LCDs if omitted)
        #[arg(long)]
        device_id: Option<String>,

        /// Duration in minutes to run cleaner (default: 30)
        #[arg(long, default_value_t = 30)]
        minutes: u32,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let system = cli.system;
    let config = cli.config.unwrap_or_else(|| default_config_path(system));
    let socket = cli.socket.unwrap_or_else(|| default_socket_path(system));

    if let Some(Commands::Lcd {
        command: LcdCommands::Clean { device_id, minutes },
    }) = cli.command
    {
        return pixel_cleaner::run_clean_command(socket, device_id, minutes);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level)),
        )
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let _pidlock = pidlock::PidLock::acquire(system)?;

    let mut manager = service::ServiceManager::new(config, socket)?;
    let restart = manager.run()?;

    if restart {
        use std::os::unix::process::CommandExt;
        let exe = std::env::current_exe()?;
        let args: Vec<String> = std::env::args().skip(1).collect();
        tracing::info!("Re-executing daemon: {} {}", exe.display(), args.join(" "));
        let err = std::process::Command::new(exe).args(args).exec();
        // exec() only returns on error
        anyhow::bail!("Failed to re-exec daemon: {err}");
    }

    Ok(())
}

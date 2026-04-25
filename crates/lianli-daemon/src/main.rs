mod aio_controller;
mod desktop_display;
mod fan_controller;
mod ipc_server;
mod openrgb_server;
mod pidlock;
mod rgb_controller;
mod service;
mod template_store;

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

fn default_config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".config")
        });
    config_dir.join("lianli").join("config.json")
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Linux daemon for Lian Li fan control and LCD streaming"
)]
struct Cli {
    /// Path to the configuration file
    #[arg(long, default_value_os_t = default_config_path())]
    config: PathBuf,

    /// Logging verbosity (error, warn, info, debug, trace)
    #[arg(long, default_value = "info")]
    log_level: String,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level)),
        )
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let _pidlock = pidlock::PidLock::acquire()?;

    let mut manager = service::ServiceManager::new(cli.config)?;
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

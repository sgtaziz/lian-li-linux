use anyhow::Context;
use clap::{Parser, Subcommand};
use lianli_shared::installation::InstallationContext;

mod worker_limits;

#[derive(Parser)]
#[command(version, about = "Lian Li Linux installation and service management")]
struct Cli {
    #[arg(long, hide = true, requires = "transfer_channel")]
    worker_destination: Option<String>,
    #[arg(long, hide = true, requires = "transfer_token")]
    transfer_channel: Option<std::path::PathBuf>,
    #[arg(long, hide = true, requires = "transfer_channel", value_parser = lianli_shared::daemon::parse_service_invocation)]
    transfer_token: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    VerifyPackageUpdate,
    #[command(hide = true)]
    RepairContainerAccess {
        #[arg(long)]
        expected_box: String,
    },
    #[command(hide = true)]
    ReadContainerDeployment {
        #[arg(long)]
        expected_box: String,
    },
    #[command(hide = true)]
    InstallContainerServices {
        #[arg(long)]
        deployment: String,
    },
    #[command(hide = true)]
    InstallContainerUserUnits {
        #[arg(long)]
        deployment: String,
        #[arg(long)]
        previous: String,
    },
    #[command(hide = true)]
    CheckContainerServices {
        #[arg(long)]
        expected_box: String,
    },
    #[command(hide = true)]
    RequestSwitch {
        #[arg(long)]
        expected_box: String,
        #[command(flatten)]
        change: ChangeArgs,
    },
    #[command(hide = true)]
    SwitchStatus {
        #[arg(long)]
        expected_box: String,
    },
    #[command(hide = true)]
    InspectContainerDeployment {
        #[arg(long)]
        deployment: String,
    },
    #[command(hide = true)]
    BoxWorker {
        #[arg(long)]
        destination: Option<String>,
        #[arg(long = "box")]
        box_name: String,
        #[arg(long, default_value = "/usr/bin/distrobox-enter")]
        distrobox_enter: std::path::PathBuf,
        #[arg(long, default_value = "/usr/bin")]
        binaries: std::path::PathBuf,
        #[arg(last = true, required = true)]
        arguments: Vec<std::ffi::OsString>,
    },
    #[command(hide = true)]
    InspectContainerDestination {
        #[arg(long = "box")]
        box_name: String,
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long)]
        working_directory: std::path::PathBuf,
        #[arg(long)]
        prepare_directory: bool,
    },
    #[command(hide = true)]
    InspectContainerIdentity {
        #[arg(long = "box")]
        box_name: String,
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long)]
        working_directory: std::path::PathBuf,
    },
    /// Print a host service unit without installing it or starting a daemon.
    DistroboxServiceUnit {
        /// Generate the desktop capture unit instead of the hardware daemon unit.
        #[arg(long)]
        desktop_session: bool,
        /// Generate a system unit running as the unprivileged host owner of the box.
        #[arg(long, requires = "system_config", conflicts_with = "desktop_session")]
        system_uid: Option<u32>,
        /// Separate, writable system-mode configuration path inside the box.
        #[arg(long, requires = "system_uid")]
        system_config: Option<std::path::PathBuf>,
        #[arg(long = "box")]
        box_name: Option<String>,
        #[arg(long, default_value = "/usr/bin/distrobox-enter")]
        distrobox_enter: std::path::PathBuf,
        #[arg(long, default_value = "/usr/bin")]
        binaries: std::path::PathBuf,
    },
    /// Inspect credentials, host-lock and USB node permissions without device I/O.
    DiagnoseRuntime {
        #[arg(long, default_value = "hidraw", value_parser = parse_hid_backend)]
        hid_backend: lianli_shared::config::HidBackend,
    },
    #[command(hide = true)]
    InspectRuntime {
        #[arg(long, value_parser = parse_hid_backend)]
        hid_backend: Option<lianli_shared::config::HidBackend>,
        #[arg(long)]
        media_tools: bool,
        #[arg(long, allow_hyphen_values = true)]
        state_directory: Option<std::path::PathBuf>,
    },
    #[command(hide = true)]
    CheckRecoveryAccess {
        #[arg(long)]
        config: std::path::PathBuf,
    },
    #[command(hide = true)]
    RecoverAutomatic,
    #[command(hide = true)]
    TriggerRecovery,
    #[command(hide = true)]
    SubmitSwitch {
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
        #[command(flatten)]
        change: ChangeArgs,
    },
    #[command(hide = true)]
    RunSwitch {
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
        #[arg(long)]
        caller_uid: u32,
        #[command(flatten)]
        change: ChangeArgs,
    },
    #[command(hide = true)]
    FinishTransfer,
    /// Switch native service mode after administrator authorization
    SwitchMode {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        carry_settings: bool,
    },
    /// Restore the previous mode after an interrupted native service switch
    RecoverSwitch,
    #[command(hide = true)]
    CheckSavedState {
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long)]
        working_directory: std::path::PathBuf,
        #[arg(long)]
        decode: bool,
    },
    #[command(hide = true)]
    InspectStartup,
    #[command(hide = true)]
    ObserveService {
        #[arg(long, value_enum)]
        scope: Scope,
    },
    #[command(hide = true)]
    PublishState,
    /// Prepare opposite-mode native settings and media under the destination account
    PrepareTransfer {
        #[arg(long, value_enum)]
        scope: Scope,
    },
    /// Remove only this operation's unpublished preparation under the current account
    DiscardTransfer {
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
    },
    #[command(hide = true)]
    SendState {
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long)]
        working_directory: std::path::PathBuf,
        #[arg(long)]
        generation: String,
    },
    #[command(hide = true)]
    SendSelectedMedia {
        #[arg(long)]
        selection: std::path::PathBuf,
    },
    #[command(hide = true)]
    ReceiveSelectedMedia {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
        #[arg(long)]
        expected_config: std::path::PathBuf,
    },
    #[command(hide = true)]
    ImportSelectedMedia {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        selection: std::path::PathBuf,
        #[arg(long)]
        expected_config: std::path::PathBuf,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
    },
    #[command(hide = true)]
    RunSelectedMediaImport {
        #[arg(long)]
        expected_instance: Option<String>,
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        selection: std::path::PathBuf,
        #[arg(long)]
        expected_config: std::path::PathBuf,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
    },
    #[command(hide = true)]
    ReceiveState {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
        #[arg(long)]
        generation: String,
    },
    #[command(hide = true)]
    RunServiceAction {
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        operation_id: String,
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long, value_enum)]
        action: Action,
    },
    /// Authorize and preflight a native service destination without stopping the current daemon
    CheckDestination {
        #[arg(long, value_enum)]
        scope: Scope,
    },
    /// Inspect the installed native destination under the executing daemon account
    #[command(hide = true)]
    InspectDestination {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        prepare_directory: bool,
    },
    /// Select one native hardware mode while all daemons are stopped (administrator only)
    SelectMode {
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long)]
        user_uid: Option<u32>,
    },
    /// Stop only the daemon belonging to this host service mode and invocation
    StopService {
        #[arg(long, value_enum, default_value = "user")]
        scope: Scope,
        #[arg(long, value_parser = lianli_shared::daemon::parse_service_invocation)]
        invocation_id: String,
    },
    /// Inspect service setup without starting a daemon or accessing hardware.
    Diagnose,
    /// Inspect saved configuration, profiles, templates and media references without changing them.
    InspectState {
        #[arg(long)]
        config: std::path::PathBuf,
        /// Working directory used by the daemon for relative template assets
        #[arg(long)]
        working_directory: std::path::PathBuf,
        /// Open every saved asset under this account, including inactive profiles and templates
        #[arg(long)]
        check_assets: bool,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum Scope {
    User,
    System,
}

#[derive(Clone, clap::ValueEnum)]
enum Action {
    Start,
    Stop,
    Restart,
}

#[derive(clap::Args)]
struct ChangeArgs {
    #[arg(long, value_enum, required_unless_present = "recover")]
    scope: Option<Scope>,
    #[arg(long, conflicts_with = "scope")]
    recover: bool,
    #[arg(long, requires = "scope", conflicts_with = "recover")]
    carry_settings: bool,
}

impl ChangeArgs {
    fn request(self) -> anyhow::Result<lianli_shared::services::ServiceChangeRequest> {
        use lianli_shared::services::{ServiceChangeRequest, ServiceScope};
        Ok(match (self.scope, self.recover) {
            (None, true) => ServiceChangeRequest::Recover {},
            (Some(scope), false) => ServiceChangeRequest::Switch {
                scope: match scope {
                    Scope::User => ServiceScope::User,
                    Scope::System => ServiceScope::System,
                },
                carry_settings: self.carry_settings,
            },
            _ => anyhow::bail!("Choose one destination mode or recovery"),
        })
    }
}

fn parse_hid_backend(value: &str) -> Result<lianli_shared::config::HidBackend, String> {
    match value {
        "hidraw" => Ok(lianli_shared::config::HidBackend::Hidraw),
        "rusb" => Ok(lianli_shared::config::HidBackend::Rusb),
        _ => Err("Choose hidraw or rusb".into()),
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command = cli.command;
    let _channel_guard =
        if let (Some(path), Some(token)) = (cli.transfer_channel, cli.transfer_token) {
            let packet = match &command {
                Command::SendState { .. }
                | Command::ReceiveState { .. }
                | Command::PublishState => true,
                Command::InspectContainerDestination { .. }
                | Command::InspectContainerIdentity { .. }
                | Command::InspectDestination { .. }
                | Command::CheckSavedState { .. }
                | Command::CheckRecoveryAccess { .. }
                | Command::DiscardTransfer { .. }
                | Command::FinishTransfer => false,
                _ => anyhow::bail!("Only state workers may receive a container channel"),
            };
            Some(lianli_control::container_channel::attach(
                &path, &token, packet,
            )?)
        } else {
            None
        };
    if let Some(destination) = cli.worker_destination {
        lianli_control::container_destination::initialize_worker(&destination)?;
    }
    if matches!(
        command,
        Command::SendState { .. } | Command::ReceiveState { .. }
    ) {
        worker_limits::apply().context("Cannot set the state-transfer memory limit")?;
    }
    match command {
        Command::ReadContainerDeployment { expected_box } => {
            lianli_control::container_change::verify_host_request(&expected_box)?;
            println!(
                "{}",
                serde_json::to_string(&lianli_control::container_deployment::load()?)?
            );
        }
        Command::InstallContainerServices { deployment } => {
            anyhow::ensure!(
                deployment.len() <= 32 * 1024,
                "Deployment record exceeds 32 KiB"
            );
            lianli_control::container_setup::install(&serde_json::from_str(&deployment)?)?;
        }
        Command::InstallContainerUserUnits {
            deployment,
            previous,
        } => {
            anyhow::ensure!(
                deployment.len() <= 32 * 1024 && previous.len() <= 32 * 1024,
                "Deployment record exceeds 32 KiB"
            );
            let previous: Option<lianli_control::container_deployment::Deployment> =
                serde_json::from_str(&previous)?;
            lianli_control::container_setup::install_user(
                &serde_json::from_str(&deployment)?,
                previous.as_ref(),
            )?;
        }
        Command::VerifyPackageUpdate => lianli_control::app_update::verify()?,
        Command::CheckContainerServices { expected_box } => {
            lianli_control::container_change::verify_host_request(&expected_box)?;
            lianli_control::container_deployment::load()?
                .context("The protected deployment is missing")?
                .inspect_installed()?;
        }
        Command::RequestSwitch {
            expected_box,
            change,
        } => {
            lianli_control::container_change::verify_host_request(&expected_box)?;
            let id = lianli_control::switch_job::start(change.request()?)?;
            println!("{}", serde_json::to_string(&id)?);
        }
        Command::RepairContainerAccess { expected_box } => {
            lianli_control::container_deployment::repair_access(&expected_box)?;
        }
        Command::SwitchStatus { expected_box } => {
            lianli_control::container_change::verify_host_request(&expected_box)?;
            println!(
                "{}",
                serde_json::to_string(&lianli_control::switch_job::read()?)?
            );
        }
        Command::InspectContainerDeployment { deployment } => {
            anyhow::ensure!(
                deployment.len() <= 32 * 1024,
                "Deployment record exceeds 32 KiB"
            );
            let deployment: lianli_control::container_deployment::Deployment =
                serde_json::from_str(&deployment)?;
            deployment.inspect_installed()?;
        }
        Command::BoxWorker {
            destination,
            box_name,
            distrobox_enter,
            binaries,
            arguments,
        } => {
            let output = lianli_control::container_channel::run(
                &box_name,
                &distrobox_enter,
                &binaries,
                &arguments,
                destination.as_deref(),
            )?;
            print!("{}", output.stdout);
            if !output.status.success() {
                if output.stderr.trim().is_empty() {
                    anyhow::bail!("Container state worker exited unsuccessfully");
                }
                anyhow::bail!("Container launcher failed: {}", output.stderr.trim());
            }
        }
        Command::InspectContainerDestination {
            box_name,
            scope,
            config,
            working_directory,
            prepare_directory,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            let result = lianli_control::destination::inspect_container(
                &box_name,
                scope,
                &config,
                &working_directory,
                prepare_directory,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::InspectContainerIdentity {
            box_name,
            scope,
            config,
            working_directory,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            let result = lianli_control::container_destination::inspect_identity(
                &box_name,
                scope,
                &config,
                &working_directory,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::DistroboxServiceUnit {
            desktop_session,
            system_uid,
            system_config,
            box_name,
            distrobox_enter,
            binaries,
        } => {
            let name = match (box_name, InstallationContext::detect()) {
                (Some(name), _) | (None, InstallationContext::Distrobox { name }) => name,
                _ => anyhow::bail!(
                    "Run inside the intended Distrobox or specify --box with its name"
                ),
            };
            if let (Some(uid), Some(config)) = (system_uid, system_config) {
                print!(
                    "{}",
                    lianli_control::distrobox_unit::generate_system(
                        &name,
                        &distrobox_enter,
                        &binaries,
                        uid,
                        &config
                    )?
                );
                return Ok(());
            }
            let generate = if desktop_session {
                lianli_control::distrobox_unit::generate_session
            } else {
                lianli_control::distrobox_unit::generate
            };
            print!("{}", generate(&name, &distrobox_enter, &binaries)?);
        }
        Command::InspectRuntime {
            hid_backend,
            media_tools,
            state_directory,
        } => {
            let mut report = lianli_control::runtime_health::inspect(hid_backend)?;
            if media_tools {
                report
                    .findings
                    .extend(lianli_control::media_health::inspect());
                report
                    .findings
                    .push(lianli_control::storage_health::inspect(
                        &std::env::temp_dir(),
                        true,
                    ));
            }
            if let Some(path) = state_directory {
                report
                    .findings
                    .push(lianli_control::storage_health::inspect(&path, false));
            }
            println!("{}", serde_json::to_string(&report)?);
        }
        Command::DiagnoseRuntime { hid_backend } => println!(
            "{}",
            serde_json::to_string(&lianli_control::runtime_health::collect_report(
                &std::env::current_exe()?,
                Some(hid_backend)
            )?)?
        ),
        Command::CheckRecoveryAccess { config } => {
            lianli_control::destination::check_recovery_access(&config)?
        }
        Command::RecoverAutomatic => println!("{}", lianli_control::automatic_recovery::run()?),
        Command::TriggerRecovery => lianli_control::automatic_recovery::trigger()?,
        Command::SubmitSwitch {
            operation_id,
            change,
        } => lianli_control::switch_job::submit(&operation_id, change.request()?)?,
        Command::RunSwitch {
            operation_id,
            caller_uid,
            change,
        } => println!(
            "{}",
            lianli_control::switch_job::run(caller_uid, &operation_id, change.request()?)?
        ),
        Command::FinishTransfer => lianli_control::transfer_cleanup::serve()?,
        Command::SwitchMode {
            scope,
            carry_settings,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                lianli_control::native_switch::execute(scope, carry_settings, |message| {
                    eprintln!("{message}");
                    Ok(())
                })?
            );
        }
        Command::RecoverSwitch => println!(
            "{}",
            lianli_control::native_switch::recover(|message| {
                eprintln!("{message}");
                Ok(())
            })?
        ),
        Command::CheckSavedState {
            config,
            working_directory,
            decode,
        } => println!(
            "{}",
            serde_json::to_string(&lianli_control::saved_state::inspect(
                &config,
                &working_directory,
                decode
            )?)?
        ),
        Command::InspectStartup => println!(
            "{}",
            serde_json::to_string(&lianli_control::service_startup::inspect()?)?
        ),
        Command::ObserveService { scope } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string(&lianli_control::authorized_service::observe(scope)?)?
            );
        }
        Command::PublishState => lianli_control::account_publication::serve()?,
        Command::PrepareTransfer { scope } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&lianli_control::state_transfer::prepare_native(
                    scope
                )?)?
            );
        }
        Command::DiscardTransfer {
            config,
            operation_id,
        } => {
            lianli_control::state_transfer::discard(&config, &operation_id)?;
        }
        Command::SendState {
            config,
            working_directory,
            generation,
        } => {
            println!(
                "{}",
                serde_json::to_string(&lianli_control::state_transfer::send(
                    &config,
                    &working_directory,
                    &generation
                )?)?
            );
        }
        Command::SendSelectedMedia { selection } => {
            lianli_control::media_import_transfer::send_file(&selection)?;
        }
        Command::ReceiveSelectedMedia {
            scope,
            operation_id,
            expected_config,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string(&lianli_control::media_import_transfer::receive_published(
                    scope,
                    &operation_id,
                    &expected_config
                )?)?
            );
        }
        Command::ImportSelectedMedia {
            scope,
            selection,
            expected_config,
            operation_id,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string(&lianli_control::media_import_launch::native(
                    scope,
                    &selection,
                    &expected_config,
                    &operation_id
                )?)?
            );
        }
        Command::RunSelectedMediaImport {
            expected_instance,
            scope,
            selection,
            expected_config,
            operation_id,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            lianli_control::media_import_worker::run(
                scope,
                &selection,
                &expected_config,
                &operation_id,
                expected_instance.as_deref(),
            )?;
        }
        Command::ReceiveState {
            scope,
            operation_id,
            generation,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string(&lianli_control::state_transfer::receive(
                    scope,
                    &operation_id,
                    &generation
                )?)?
            );
        }
        Command::RunServiceAction {
            operation_id,
            scope,
            action,
        } => {
            use lianli_shared::services::{ServiceAction, ServiceActionRequest, ServiceScope};
            let request = ServiceActionRequest {
                scope: match scope {
                    Scope::User => ServiceScope::User,
                    Scope::System => ServiceScope::System,
                },
                action: match action {
                    Action::Start => ServiceAction::Start,
                    Action::Stop => ServiceAction::Stop,
                    Action::Restart => ServiceAction::Restart,
                },
            };
            println!(
                "{}",
                lianli_control::operation_job::run(&operation_id, request)?
            );
        }
        Command::CheckDestination { scope } => {
            anyhow::ensure!(
                InstallationContext::detect() == InstallationContext::Native,
                "Automatic system/user switching requires the native host application"
            );
            let caller = lianli_control::account::Account::authorized_caller()?;
            let (account, scope) = match scope {
                Scope::User => (caller, lianli_shared::services::ServiceScope::User),
                Scope::System => (
                    lianli_control::account::Account::system()?,
                    lianli_shared::services::ServiceScope::System,
                ),
            };
            let _operation = lianli_control::reservation::ServiceOperationLock::acquire(
                &InstallationContext::Native,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&lianli_control::destination::preflight(
                    &account, scope
                )?)?
            );
        }
        Command::InspectDestination {
            scope,
            prepare_directory,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            println!(
                "{}",
                serde_json::to_string(&lianli_control::destination::inspect(
                    scope,
                    prepare_directory
                )?)?
            );
        }
        Command::SelectMode { scope, user_uid } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            let selection = lianli_control::service_selection::select(scope, user_uid)?;
            println!("{}", serde_json::to_string_pretty(&selection)?);
        }
        Command::StopService {
            scope,
            invocation_id,
        } => {
            let scope = match scope {
                Scope::User => lianli_shared::services::ServiceScope::User,
                Scope::System => lianli_shared::services::ServiceScope::System,
            };
            lianli_control::service_stop::stop_in_scope(
                &InstallationContext::detect(),
                scope,
                &invocation_id,
            )?;
            println!("The service daemon exited.");
        }
        Command::Diagnose => {
            let context = InstallationContext::detect();
            let report = lianli_control::services::inspect(&context);
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::InspectState {
            config,
            working_directory,
            check_assets,
        } => {
            let snapshot = lianli_control::state::StateSnapshot::read(&config, &working_directory)?;
            if check_assets {
                let access =
                    snapshot.check_assets(&lianli_control::media_staging::CopyControl::new(
                        std::time::Duration::from_secs(30),
                    ))?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "state": snapshot.summary, "assets": access
                    }))?
                );
                anyhow::ensure!(
                    snapshot.summary.issue_count == 0 && access.failed == 0,
                    "Saved state or asset access failed validation. See the JSON report"
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&snapshot.summary)?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod switch_cli_tests {
    use super::*;

    #[test]
    fn system_box_unit_requires_an_owner_and_configuration_without_a_desktop_helper() {
        let base = [
            "lianli-control",
            "distrobox-service-unit",
            "--box",
            "fixture",
        ];
        for args in [
            &[][..],
            &["--desktop-session"],
            &[
                "--system-uid",
                "1000",
                "--system-config",
                "/state/config.json",
            ],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(args.iter().copied())).is_ok());
        }
        for args in [
            &["--system-uid", "1000"][..],
            &["--system-config", "/state/config.json"],
            &[
                "--system-uid",
                "1000",
                "--system-config",
                "/state/config.json",
                "--desktop-session",
            ],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(args.iter().copied())).is_err());
        }
    }

    #[test]
    fn switch_submission_accepts_one_mode_or_recovery_without_executing_it() {
        let base = [
            "lianli-control",
            "submit-switch",
            "--operation-id",
            "0123456789abcdef0123456789abcdef",
        ];
        for args in [
            &["--scope", "user"][..],
            &["--scope", "system", "--carry-settings"],
            &["--recover"],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(args.iter().copied())).is_ok());
        }
        for args in [
            &[][..],
            &["--scope", "root"],
            &["--recover", "--carry-settings"],
            &["--recover", "--scope", "user"],
        ] {
            assert!(
                Cli::try_parse_from(base.into_iter().chain(args.iter().copied())).is_err(),
                "Accepted invalid arguments: {args:?}"
            );
        }
    }
}

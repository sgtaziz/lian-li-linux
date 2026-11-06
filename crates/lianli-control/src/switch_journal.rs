use crate::reservation::{HardwareReservation, ServiceOperationLock};
use crate::state_transfer::PreparedTransfer;
use anyhow::{ensure, Context, Result};
use lianli_shared::installation::InstallationContext;
use lianli_shared::services::ServiceSelection;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const DIRECTORY: &str = "/var/lib/lianli-control";
const ACTIVE: &str = "switch.json";
const LAST: &str = "last-switch.json";
const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Startup {
    Disabled,
    Enabled,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<crate::container_destination::Route>,
    pub id: String,
    pub caller_name: String,
    pub source: ServiceSelection,
    pub destination: ServiceSelection,
    pub source_config: PathBuf,
    pub destination_config: PathBuf,
    #[serde(default)]
    pub source_working_directory: Option<PathBuf>,
    #[serde(default)]
    pub destination_working_directory: Option<PathBuf>,
    pub source_was_running: bool,
    pub previous_selection: Option<ServiceSelection>,
    pub user_startup: Startup,
    pub system_startup: Startup,
    pub carry_settings: bool,
    #[serde(default)]
    pub user_runtime: Option<lianli_shared::daemon::FileIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Ready,
    StoppingSource,
    SourceStopped,
    Publishing,
    Published,
    SelectingDestination,
    StartingDestination,
    VerifyingDestination,
    Complete,
    StoppingDestination,
    RestoringDestination,
    RestoringStartup,
    StartingSource,
    VerifyingSource,
    RolledBack,
    RecoveryRequired,
    RestoringSystem,
    SystemRestored,
    StoppingRecoveredSystem,
}

impl Phase {
    fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::RolledBack)
    }

    fn permits(self, next: Self) -> bool {
        use Phase::*;
        if self.terminal() {
            return false;
        }
        if next == RecoveryRequired {
            return true;
        }
        if next == RestoringSystem {
            return !matches!(self, Preparing | Ready);
        }
        matches!(
            (self, next),
            (Preparing, Ready | RolledBack)
                | (Ready, StoppingSource | RolledBack)
                | (StoppingSource, SourceStopped)
                | (SourceStopped, Publishing | Published)
                | (Publishing, Published)
                | (Published, SelectingDestination)
                | (SelectingDestination, StartingDestination)
                | (StartingDestination, VerifyingDestination)
                | (VerifyingDestination, Complete)
                | (StoppingDestination, RestoringDestination)
                | (RestoringDestination, RestoringStartup)
                | (RestoringStartup, StartingSource)
                | (StartingSource, VerifyingSource)
                | (VerifyingSource, RolledBack)
                | (RestoringSystem, SystemRestored)
                | (SystemRestored, StoppingRecoveredSystem)
                | (StoppingRecoveredSystem, StoppingDestination)
                | (
                    RecoveryRequired
                        | StoppingSource
                        | SourceStopped
                        | Publishing
                        | Published
                        | SelectingDestination
                        | StartingDestination
                        | VerifyingDestination,
                    StoppingDestination
                )
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    version: u32,
    #[serde(default)]
    boot_id: Option<String>,
    pub intent: Intent,
    pub phase: Phase,
    pub prepared: Option<PreparedTransfer>,
    pub destination_backup: Option<String>,
    pub rollback_backup: Option<String>,
    pub destination_instance: Option<String>,
    pub failure: Option<String>,
    #[serde(default)]
    system_start_boot: Option<String>,
}

impl Record {
    fn submit_system_start(&self, boot: &str) -> Result<Self> {
        ensure!(
            self.phase == Phase::RestoringSystem,
            "Record system restoration before starting it"
        );
        ensure!(self.system_start_boot.as_deref() != Some(boot),
            "System recovery already submitted startup during this boot. Inspect the service journal without replaying an uncertain start");
        let mut next = self.clone();
        next.system_start_boot = Some(boot.into());
        next.validate()?;
        Ok(next)
    }

    pub(crate) fn system_restoration(&self, current_boot: &str) -> Result<(Startup, bool)> {
        ensure!(
            self.intent.source.scope == lianli_shared::services::ServiceScope::System,
            "Offline recovery can only restore a previous system source"
        );
        let policy = crate::service_startup::Policy {
            user: self.intent.user_startup,
            system: self.intent.system_startup,
        }
        .restore_for_boot(self.boot_id.as_deref(), current_boot)?;
        Ok((policy.system, self.source_should_run(current_boot, None)?))
    }

    fn source_should_run(
        &self,
        current_boot: &str,
        current_runtime: Option<&lianli_shared::daemon::FileIdentity>,
    ) -> Result<bool> {
        let original_boot = self.boot_id.as_deref().context(
            "The original boot is unknown. Preserve the journal and inspect the intended running state before recovery",
        )?;
        if original_boot == current_boot
            && self.intent.source.scope == lianli_shared::services::ServiceScope::User
        {
            let original = self.intent.user_runtime.as_ref().context("The original user runtime is unknown. Inspect the journal before restarting its source service")?;
            if Some(original) != current_runtime {
                return Ok(self.intent.user_startup == Startup::Enabled);
            }
        }
        if original_boot == current_boot {
            return Ok(self.intent.source_was_running);
        }
        let startup = match self.intent.source.scope {
            lianli_shared::services::ServiceScope::User => self.intent.user_startup,
            lianli_shared::services::ServiceScope::System => self.intent.system_startup,
        };
        Ok(startup == Startup::Enabled)
    }

    fn restoration_policy(
        &self,
        current_boot: &str,
        current_runtime: Option<&lianli_shared::daemon::FileIdentity>,
    ) -> Result<crate::service_startup::Policy> {
        let mut policy = crate::service_startup::Policy {
            user: self.intent.user_startup,
            system: self.intent.system_startup,
        }
        .restore_for_boot(self.boot_id.as_deref(), current_boot)?;
        if policy.user == Startup::Runtime {
            let original = self.intent.user_runtime.as_ref().context("The original user runtime is unknown. Inspect the journal before restoring runtime-only startup")?;
            if Some(original) != current_runtime {
                policy.user = Startup::Disabled;
            }
        }
        Ok(policy)
    }

    fn new(intent: Intent) -> Result<Self> {
        let record = Self {
            version: 1,
            boot_id: Some(crate::service_startup::boot_id()?),
            intent,
            phase: Phase::Preparing,
            prepared: None,
            destination_backup: None,
            rollback_backup: None,
            destination_instance: None,
            failure: None,
            system_start_boot: None,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<()> {
        if let Some(boot) = &self.system_start_boot {
            lianli_shared::daemon::parse_service_invocation(boot).map_err(anyhow::Error::msg)?;
        }
        if matches!(
            self.phase,
            Phase::RestoringSystem | Phase::SystemRestored | Phase::StoppingRecoveredSystem
        ) {
            ensure!(
                self.intent.source.scope == lianli_shared::services::ServiceScope::System,
                "Deferred system recovery requires a system source"
            );
        }
        ensure!(self.version == 1, "Unsupported switch journal version");
        if let Some(id) = &self.boot_id {
            lianli_shared::daemon::parse_service_invocation(id).map_err(anyhow::Error::msg)?;
        }
        if let Some(identity) = &self.intent.user_runtime {
            ensure!(
                identity.device.parse::<u64>().is_ok() && identity.inode.parse::<u64>().is_ok(),
                "Invalid original user runtime identity"
            );
        }
        crate::service_startup::Policy {
            user: self.intent.user_startup,
            system: self.intent.system_startup,
        }
        .validate()?;
        lianli_shared::daemon::parse_service_invocation(&self.intent.id)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.intent.source.scope != self.intent.destination.scope
                && self.intent.source.uid != 0
                && self.intent.destination.uid != 0,
            "Switch journal must identify opposite modes under unprivileged accounts"
        );
        if let Some(route) = &self.intent.container {
            route.validate()?;
            ensure!(
                self.intent.source.uid == self.intent.destination.uid,
                "Container service modes must belong to the same host owner"
            );
            for (selection, config, working) in [
                (
                    self.intent.source,
                    &self.intent.source_config,
                    &self.intent.source_working_directory,
                ),
                (
                    self.intent.destination,
                    &self.intent.destination_config,
                    &self.intent.destination_working_directory,
                ),
            ] {
                let (expected_config, expected_working) = route.paths(selection.scope);
                ensure!(
                    config == expected_config && working.as_deref() == Some(expected_working),
                    "Container route differs from the recorded service destination"
                );
            }
        } else {
            ensure!(
                self.intent.source.uid != self.intent.destination.uid,
                "Native switching requires distinct unprivileged service accounts"
            );
        }
        ensure!(
            !self.intent.caller_name.is_empty()
                && self.intent.caller_name.len() <= 256
                && !self.intent.caller_name.chars().any(char::is_control),
            "Invalid switch caller identity"
        );
        for config in [&self.intent.source_config, &self.intent.destination_config] {
            ensure!(
                config.is_absolute()
                    && config.as_os_str().len() <= 4096
                    && !config.as_os_str().as_encoded_bytes().contains(&0)
                    && !config
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir)),
                "Invalid switch configuration path"
            );
            crate::container_destination::validate_config(config)?;
        }
        ensure!(
            self.intent.source_config != self.intent.destination_config,
            "Switch destinations must use distinct configuration files"
        );
        for path in [
            &self.intent.source_working_directory,
            &self.intent.destination_working_directory,
        ]
        .into_iter()
        .flatten()
        {
            ensure!(
                path.is_absolute()
                    && path.as_os_str().len() <= 4096
                    && !path.as_os_str().as_encoded_bytes().contains(&0)
                    && !path
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir)),
                "Invalid service working directory in the switch journal"
            );
        }
        if let Some(previous) = self.intent.previous_selection {
            ensure!(
                previous == self.intent.source || previous == self.intent.destination,
                "Previous service selection belongs to another account"
            );
        }
        if let Some(prepared) = &self.prepared {
            crate::state_transfer::validate_preparation(
                prepared,
                &self.intent.destination_config,
                &self.intent.id,
            )?;
            ensure!(
                self.intent.carry_settings
                    && prepared.id == self.intent.id
                    && prepared.destination_uid == self.intent.destination.uid
                    && prepared.config_path == self.intent.destination_config
                    && prepared.decode_validated
                    && prepared.state_generation.is_some()
                    && prepared.media_generation.is_some(),
                "Prepared transfer differs from the switch intent"
            );
        }
        for backup in [&self.destination_backup, &self.rollback_backup]
            .into_iter()
            .flatten()
        {
            crate::state_transaction::validate_backup(backup)?;
        }
        ensure!(
            self.failure
                .as_ref()
                .is_none_or(|message| message.len() <= 4096),
            "Switch error exceeds 4 KiB"
        );
        ensure!(
            self.destination_instance
                .as_ref()
                .is_none_or(|id| !id.is_empty() && id.len() <= 256),
            "Invalid destination instance identity"
        );
        if self.intent.carry_settings
            && matches!(
                self.phase,
                Phase::Ready
                    | Phase::StoppingSource
                    | Phase::SourceStopped
                    | Phase::Publishing
                    | Phase::Published
                    | Phase::SelectingDestination
                    | Phase::StartingDestination
                    | Phase::VerifyingDestination
                    | Phase::Complete
            )
        {
            ensure!(
                self.prepared.is_some(),
                "Complete destination preparation before stopping the source"
            );
        }
        if self.intent.carry_settings
            && matches!(
                self.phase,
                Phase::Published
                    | Phase::SelectingDestination
                    | Phase::StartingDestination
                    | Phase::VerifyingDestination
                    | Phase::Complete
            )
        {
            ensure!(
                self.destination_backup.is_some(),
                "Record the destination backup before completing publication"
            );
        }
        if self.phase == Phase::Complete {
            ensure!(
                self.destination_instance.is_some(),
                "Verify the destination daemon before completing the switch"
            );
        }
        ensure!(
            self.destination_backup.is_none() || self.prepared.is_some(),
            "A destination backup requires its prepared transfer"
        );
        ensure!(
            self.rollback_backup.is_none() || self.destination_backup.is_some(),
            "A rollback backup requires the original destination backup"
        );
        Ok(())
    }

    fn advance(&self, phase: Phase) -> Result<Self> {
        ensure!(
            self.phase.permits(phase),
            "Invalid switch transition {:?} -> {phase:?}",
            self.phase
        );
        let mut next = self.clone();
        next.phase = phase;
        next.validate()?;
        Ok(next)
    }
}

pub struct Journal<'a> {
    operation: &'a ServiceOperationLock,
    store: Store,
    record: Record,
}

impl<'a> Journal<'a> {
    pub fn create(operation: &'a ServiceOperationLock, intent: Intent) -> Result<Self> {
        authorize(operation)?;
        let record = Record::new(intent)?;
        let store = Store::open(Path::new(DIRECTORY), true)?;
        ensure!(
            store.read()?.is_none(),
            "Recover the pending service switch before starting another"
        );
        store.write(&record)?;
        Ok(Self {
            operation,
            store,
            record,
        })
    }

    pub fn load(operation: &'a ServiceOperationLock) -> Result<Option<Self>> {
        authorize(operation)?;
        if fs::symlink_metadata(DIRECTORY)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(None);
        }
        let store = Store::open(Path::new(DIRECTORY), true)?;
        Ok(store.read()?.map(|record| Self {
            operation,
            store,
            record,
        }))
    }

    pub fn record(&self) -> &Record {
        &self.record
    }

    pub fn pause_launches(&self) -> Result<()> {
        self.verify_record()?;
        ensure!(
            matches!(
                self.record.phase,
                Phase::StoppingSource
                    | Phase::StoppingDestination
                    | Phase::RestoringDestination
                    | Phase::RecoveryRequired
                    | Phase::RestoringSystem
                    | Phase::StoppingRecoveredSystem
            ),
            "Record the service stop or recovery phase before pausing daemon startup"
        );
        crate::service_selection::pause(&self.record.intent.id, self.operation)
    }

    pub fn select_destination(
        &self,
        caller: &crate::account::Account,
        hardware: &HardwareReservation,
    ) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase == Phase::SelectingDestination,
            "Complete publication before selecting the destination daemon"
        );
        self.verify_caller(caller)?;
        let desired = match self.record.intent.destination.scope {
            lianli_shared::services::ServiceScope::User => crate::service_startup::Policy {
                user: Startup::Enabled,
                system: Startup::Disabled,
            },
            lianli_shared::services::ServiceScope::System => crate::service_startup::Policy {
                user: Startup::Disabled,
                system: Startup::Enabled,
            },
        };
        crate::service_startup::apply(
            caller,
            self.operation,
            hardware,
            &self.record.intent.id,
            desired,
        )?;
        crate::service_selection::resume(
            Some(self.record.intent.destination),
            &self.record.intent.id,
            self.operation,
            hardware,
        )
    }

    pub fn restore_selection(
        &self,
        caller: &crate::account::Account,
        hardware: &HardwareReservation,
    ) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase == Phase::RestoringStartup,
            "Complete destination restoration before restoring startup selection"
        );
        self.verify_caller(caller)?;
        let desired = self.restoration_policy()?;
        crate::service_startup::apply(
            caller,
            self.operation,
            hardware,
            &self.record.intent.id,
            desired,
        )?;
        crate::service_selection::resume(
            self.record.intent.previous_selection,
            &self.record.intent.id,
            self.operation,
            hardware,
        )
    }

    pub(crate) fn restore_system_selection(&self, hardware: &HardwareReservation) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase == Phase::RestoringSystem,
            "Record system restoration before selecting it"
        );
        let (startup, _) = self
            .record
            .system_restoration(&crate::service_startup::boot_id()?)?;
        crate::service_startup::restore_system(
            self.operation,
            hardware,
            &self.record.intent.id,
            startup,
        )?;
        crate::service_selection::resume(
            Some(self.record.intent.source),
            &self.record.intent.id,
            self.operation,
            hardware,
        )
    }

    pub(crate) fn resume_live_system(
        &self,
        expected: &lianli_shared::daemon::FileIdentity,
    ) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase == Phase::RestoringSystem,
            "Record system restoration before reopening startup"
        );
        crate::service_selection::resume_live_system(
            self.record.intent.source,
            &self.record.intent.id,
            self.operation,
            || {
                self.verify_record()?;
                crate::system_recovery::verify_live_source(&self.record, expected)
            },
        )
    }

    pub(crate) fn submit_system_start(&mut self, boot: &str) -> Result<()> {
        self.replace(self.record.submit_system_start(boot)?)
    }

    pub(crate) fn recovered_system_stopped(
        &mut self,
        hardware: &HardwareReservation,
    ) -> Result<()> {
        hardware.verify()?;
        let mut next = self.record.advance(Phase::StoppingDestination)?;
        next.system_start_boot = None;
        self.replace(next)
    }

    pub(crate) fn restoration_policy(&self) -> Result<crate::service_startup::Policy> {
        self.record.restoration_policy(
            &crate::service_startup::boot_id()?,
            self.current_user_runtime()?.as_ref(),
        )
    }

    pub(crate) fn source_should_run(&self) -> Result<bool> {
        self.record.source_should_run(
            &crate::service_startup::boot_id()?,
            self.current_user_runtime()?.as_ref(),
        )
    }

    pub(crate) fn current_user_runtime(
        &self,
    ) -> Result<Option<lianli_shared::daemon::FileIdentity>> {
        let user = if self.record.intent.source.scope == lianli_shared::services::ServiceScope::User
        {
            self.record.intent.source
        } else {
            self.record.intent.destination
        };
        crate::user_runtime::identity(user.uid)
    }

    fn verify_caller(&self, caller: &crate::account::Account) -> Result<()> {
        let user = if self.record.intent.source.scope == lianli_shared::services::ServiceScope::User
        {
            self.record.intent.source
        } else {
            self.record.intent.destination
        };
        ensure!(
            caller.uid == user.uid && caller.name == self.record.intent.caller_name,
            "The startup account differs from the switch caller"
        );
        Ok(())
    }

    pub fn publish_destination(
        &mut self,
        account: &crate::account::Account,
        hardware: &HardwareReservation,
    ) -> Result<()> {
        ensure!(
            account.uid == self.record.intent.destination.uid,
            "Publication account differs from the switch destination"
        );
        let prepared = self
            .record
            .prepared
            .clone()
            .context("The switch has no prepared transfer")?;
        self.advance(Phase::Publishing)?;
        let operation = self.operation;
        let completed = crate::account_publication::execute(
            account,
            crate::account_publication::Action::Publish(Box::new(prepared)),
            operation,
            hardware,
            |backup| {
                self.backup(backup.context("Publication did not identify its rollback backup")?)
            },
        )?;
        ensure!(
            completed == self.record.destination_backup,
            "Destination publication returned a different backup"
        );
        self.advance(Phase::Published)
    }

    pub fn restore_destination(
        &mut self,
        account: &crate::account::Account,
        hardware: &HardwareReservation,
    ) -> Result<()> {
        ensure!(
            account.uid == self.record.intent.destination.uid,
            "Restoration account differs from the switch destination"
        );
        if self.record.phase != Phase::RestoringDestination {
            self.advance(Phase::RestoringDestination)?;
        }
        let operation = self.operation;
        let config = self.record.intent.destination_config.clone();
        crate::account_publication::execute(
            account,
            crate::account_publication::Action::Recover {
                config: config.clone(),
            },
            operation,
            hardware,
            |backup| {
                ensure!(backup.is_none(), "Unexpected backup before state recovery");
                Ok(())
            },
        )?;
        if let Some(backup) = &self.record.destination_backup {
            let action = match &self.record.rollback_backup {
                Some(undo) => crate::account_publication::Action::ResumeRestore {
                    config,
                    backup: backup.clone(),
                    undo: undo.clone(),
                },
                None => crate::account_publication::Action::Restore {
                    config,
                    backup: backup.clone(),
                },
            };
            let completed = crate::account_publication::execute(
                account,
                action,
                operation,
                hardware,
                |backup| {
                    self.backup(backup.context("Restoration did not identify its undo backup")?)
                },
            )?;
            ensure!(
                completed == self.record.rollback_backup,
                "Destination restoration returned a different backup"
            );
        }
        self.advance(Phase::RestoringStartup)
    }

    pub fn advance(&mut self, phase: Phase) -> Result<()> {
        self.replace(self.record.advance(phase)?)
    }

    pub fn prepared(&mut self, prepared: PreparedTransfer) -> Result<()> {
        ensure!(
            self.record.phase == Phase::Preparing && self.record.prepared.is_none(),
            "Preparation was already recorded"
        );
        let mut next = self.record.clone();
        next.prepared = Some(prepared);
        self.replace(next)
    }

    pub fn backup(&mut self, backup: &str) -> Result<()> {
        let mut next = self.record.clone();
        let target = match next.phase {
            Phase::Publishing => &mut next.destination_backup,
            Phase::RestoringDestination => &mut next.rollback_backup,
            _ => anyhow::bail!("The switch is not preparing state publication or restoration"),
        };
        ensure!(
            target.as_deref().is_none_or(|previous| previous == backup),
            "Switch backup identity changed"
        );
        *target = Some(backup.into());
        self.replace(next)
    }

    pub fn verified_destination(&mut self, instance: &str) -> Result<()> {
        ensure!(
            self.record.phase == Phase::VerifyingDestination,
            "Destination is not awaiting verification"
        );
        let mut next = self.record.clone();
        next.destination_instance = Some(instance.into());
        self.replace(next)
    }

    pub fn failure(&mut self, error: &str) -> Result<()> {
        let mut next = self.record.clone();
        let mut end = error.len().min(4096);
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        next.failure = Some(error[..end].into());
        self.replace(next)
    }

    pub fn cleanup_preparation(&mut self, account: &crate::account::Account) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase.terminal(),
            "Finalize the service switch before removing preparation files"
        );
        if let Some(prepared) = &self.record.prepared {
            if let Err(error) =
                crate::transfer_cleanup::finish_as(account, prepared, self.operation)
            {
                self.failure(&format!(
                    "Service mode finalized. Preparation cleanup failed: {error:#}"
                ))?;
                return Err(error);
            }
        }
        Ok(())
    }

    fn replace(&mut self, next: Record) -> Result<()> {
        self.verify_record()?;
        self.store.write(&next)?;
        self.record = next;
        Ok(())
    }

    fn verify_record(&self) -> Result<()> {
        authorize(self.operation)?;
        ensure!(
            self.store.read()?.as_ref() == Some(&self.record),
            "Switch journal changed outside its coordinator"
        );
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        self.verify_record()?;
        ensure!(
            self.record.phase.terminal(),
            "Cannot archive an incomplete service switch"
        );
        self.store.archive()
    }
}

fn authorize(operation: &ServiceOperationLock) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0
            && InstallationContext::detect() == InstallationContext::Native,
        "Switch journaling requires its native authorized coordinator"
    );
    operation.verify()
}

struct Store {
    directory: File,
    path: PathBuf,
    root_owned: bool,
}

impl Store {
    fn open(path: &Path, root_owned: bool) -> Result<Self> {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => File::open(path.parent().context("Invalid switch journal directory")?)?
                .sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("Creating switch journal directory"),
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let store = Self {
            directory,
            path: path.into(),
            root_owned,
        };
        store.verify()?;
        Ok(store)
    }

    fn pinned(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd())).join(name)
    }

    fn verify(&self) -> Result<()> {
        let held = self.directory.metadata()?;
        let current = fs::symlink_metadata(&self.path)?;
        ensure!(
            held.uid()
                == if self.root_owned {
                    0
                } else {
                    unsafe { libc::geteuid() }
                }
                && held.mode() & 0o077 == 0
                && current.is_dir()
                && (held.dev(), held.ino()) == (current.dev(), current.ino()),
            "Switch journal directory {} is not private, owned by its coordinator, or was replaced",
            self.path.display()
        );
        Ok(())
    }

    fn read(&self) -> Result<Option<Record>> {
        self.verify()?;
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(self.pinned(ACTIVE))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Reading pending switch journal"),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.len() <= MAX_BYTES as u64
                && metadata.nlink() == 1
                && metadata.uid() == self.directory.metadata()?.uid()
                && metadata.mode() & 0o077 == 0,
            "Invalid switch journal file. Preserve it for recovery"
        );
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BYTES, "Switch journal exceeds 256 KiB");
        let record: Record = serde_json::from_slice(&bytes)
            .context("Corrupt switch journal. Preserve it for recovery")?;
        record.validate()?;
        Ok(Some(record))
    }

    fn write(&self, record: &Record) -> Result<()> {
        self.verify()?;
        record.validate()?;
        let bytes = serde_json::to_vec(record)?;
        ensure!(bytes.len() <= MAX_BYTES, "Switch journal exceeds 256 KiB");
        let mut file = tempfile::NamedTempFile::new_in(self.pinned(""))?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        self.verify()?;
        file.persist(self.pinned(ACTIVE))?;
        self.directory.sync_all()?;
        Ok(())
    }

    fn archive(&self) -> Result<()> {
        self.verify()?;
        fs::rename(self.pinned(ACTIVE), self.pinned(LAST))?;
        self.directory.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_system_recovery_preserves_checkpoints_and_does_not_replay_startup() {
        let root = tempfile::tempdir().unwrap();
        let mut requested = intent(root.path());
        std::mem::swap(&mut requested.source, &mut requested.destination);
        requested.user_startup = Startup::Disabled;
        requested.system_startup = Startup::Enabled;
        requested.user_runtime = None;
        let mut record = Record::new(requested).unwrap();
        let boot = record.boot_id.clone().unwrap();
        assert_eq!(
            record.system_restoration(&boot).unwrap(),
            (Startup::Enabled, true)
        );
        assert_eq!(
            record.system_restoration("another-boot").unwrap(),
            (Startup::Enabled, true)
        );
        assert!(record.advance(Phase::RestoringSystem).is_err());
        record.phase = Phase::Publishing;
        record = record.advance(Phase::RestoringSystem).unwrap();
        record = record.submit_system_start(&boot).unwrap();
        let store = Store::open(&root.path().join("journal"), false).unwrap();
        store.write(&record).unwrap();
        let reopened = store.read().unwrap().unwrap();
        assert!(reopened.submit_system_start(&boot).is_err());
        assert!(reopened
            .submit_system_start("123456789abcdef0123456789abcdef0")
            .is_ok());
        record = reopened.advance(Phase::SystemRestored).unwrap();
        assert!(record.advance(Phase::RolledBack).is_err());
        record = record.advance(Phase::StoppingRecoveredSystem).unwrap();
        record = record.advance(Phase::StoppingDestination).unwrap();
        record = record.advance(Phase::RestoringDestination).unwrap();
        record = record.advance(Phase::RestoringStartup).unwrap();
        record = record.advance(Phase::StartingSource).unwrap();
        record = record.advance(Phase::VerifyingSource).unwrap();
        record.advance(Phase::RolledBack).unwrap();
        let user = Record::new(intent(root.path())).unwrap();
        assert!(user.system_restoration(&boot).is_err());
        let mut user = user;
        user.phase = Phase::StoppingSource;
        assert!(user.advance(Phase::RestoringSystem).is_err());
    }
    use lianli_shared::services::ServiceScope;

    fn intent(root: &Path) -> Intent {
        Intent {
            container: None,
            id: "0123456789abcdef0123456789abcdef".into(),
            caller_name: "fixture".into(),
            source: ServiceSelection {
                scope: ServiceScope::User,
                uid: 1000,
            },
            destination: ServiceSelection {
                scope: ServiceScope::System,
                uid: 900,
            },
            source_config: root.join("source/config.json"),
            destination_config: root.join("destination/config.json"),
            source_working_directory: Some(root.join("source")),
            destination_working_directory: Some(root.join("destination")),
            source_was_running: true,
            previous_selection: None,
            user_startup: Startup::Runtime,
            system_startup: Startup::Disabled,
            carry_settings: false,
            user_runtime: Some(lianli_shared::daemon::FileIdentity {
                device: "1".into(),
                inode: "2".into(),
            }),
        }
    }

    #[test]
    fn container_journals_require_one_owner_and_exact_mode_paths() {
        let root = tempfile::tempdir().unwrap();
        let mut intent = intent(root.path());
        intent.source_config = root.path().join("source/settings");
        intent.destination_config = root.path().join("destination/settings.conf");
        intent.destination.uid = intent.source.uid;
        assert!(Record::new(intent.clone()).is_err());
        intent.container = Some(crate::container_destination::Route {
            launch: crate::container_destination::Launch {
                name: "release-box".into(),
                host_enter: "/usr/bin/distrobox-enter".into(),
                binaries: "/opt/lianli".into(),
            },
            user_config: intent.source_config.clone(),
            system_config: intent.destination_config.clone(),
            user_working_directory: intent.source_working_directory.clone().unwrap(),
            system_working_directory: intent.destination_working_directory.clone().unwrap(),
        });
        let record = Record::new(intent.clone()).unwrap();
        let restored: Record =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(record.intent, restored.intent);
        for change in 0..5 {
            let mut changed = intent.clone();
            match change {
                0 => changed.destination.uid += 1,
                1 => changed.destination.scope = changed.source.scope,
                2 => changed.destination_working_directory = None,
                3 => changed.source_config = root.path().join("elsewhere/config.json"),
                _ => {
                    changed.container.as_mut().unwrap().system_config =
                        changed.source_config.clone()
                }
            }
            assert!(Record::new(changed).is_err(), "change {change}");
        }
        std::mem::swap(&mut intent.source, &mut intent.destination);
        std::mem::swap(&mut intent.source_config, &mut intent.destination_config);
        std::mem::swap(
            &mut intent.source_working_directory,
            &mut intent.destination_working_directory,
        );
        Record::new(intent).unwrap();
    }

    #[test]
    fn older_journals_without_boot_identity_preserve_state_but_cannot_restore_runtime_startup() {
        let root = tempfile::tempdir().unwrap();
        let record = Record::new(intent(root.path())).unwrap();
        let mut value = serde_json::to_value(&record).unwrap();
        assert!(value["intent"].get("container").is_none());
        value.as_object_mut().unwrap().remove("boot_id");
        let older: Record = serde_json::from_value(value).unwrap();
        older.validate().unwrap();
        assert!(older.boot_id.is_none());
        assert!(older.source_should_run("current", None).is_err());
        assert_eq!(older.intent, record.intent);
        assert!(crate::service_startup::Policy {
            user: older.intent.user_startup,
            system: older.intent.system_startup
        }
        .restore_for_boot(older.boot_id.as_deref(), "current")
        .is_err());
        let mut invalid = record;
        invalid.boot_id = Some("invalid".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn recovery_after_reboot_uses_startup_policy_instead_of_the_previous_process_state() {
        let root = tempfile::tempdir().unwrap();
        for scope in [ServiceScope::User, ServiceScope::System] {
            for startup in [Startup::Disabled, Startup::Runtime, Startup::Enabled] {
                for was_running in [false, true] {
                    let mut intent = intent(root.path());
                    if scope == ServiceScope::System {
                        std::mem::swap(&mut intent.source, &mut intent.destination);
                    }
                    intent.user_startup = if scope == ServiceScope::User {
                        startup
                    } else {
                        Startup::Disabled
                    };
                    intent.system_startup = if scope == ServiceScope::System {
                        startup
                    } else {
                        Startup::Disabled
                    };
                    intent.source_was_running = was_running;
                    let mut record = Record::new(intent).unwrap();
                    let original = record.boot_id.clone().unwrap();
                    for phase in [
                        Phase::RestoringStartup,
                        Phase::StartingSource,
                        Phase::VerifyingSource,
                    ] {
                        record.phase = phase;
                        assert_eq!(
                            record
                                .source_should_run(&original, record.intent.user_runtime.as_ref())
                                .unwrap(),
                            was_running
                        );
                        assert_eq!(
                            record.source_should_run("another-boot", None).unwrap(),
                            startup == Startup::Enabled
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn logout_expires_user_runtime_startup_without_changing_system_runtime_startup() {
        let root = tempfile::tempdir().unwrap();
        let mut record = Record::new(intent(root.path())).unwrap();
        let boot = record.boot_id.clone().unwrap();
        let replacement = lianli_shared::daemon::FileIdentity {
            device: "1".into(),
            inode: "3".into(),
        };
        assert_eq!(
            record
                .restoration_policy(&boot, record.intent.user_runtime.as_ref())
                .unwrap()
                .user,
            Startup::Runtime
        );
        assert!(record
            .source_should_run(&boot, record.intent.user_runtime.as_ref())
            .unwrap());
        for current in [None, Some(&replacement)] {
            assert_eq!(
                record.restoration_policy(&boot, current).unwrap().user,
                Startup::Disabled
            );
            assert!(!record.source_should_run(&boot, current).unwrap());
        }
        record.intent.user_startup = Startup::Enabled;
        record.intent.source_was_running = false;
        assert!(record.source_should_run(&boot, Some(&replacement)).unwrap());
        assert_eq!(
            record
                .restoration_policy(&boot, Some(&replacement))
                .unwrap()
                .user,
            Startup::Enabled
        );
        record.intent.user_startup = Startup::Disabled;
        record.intent.source_was_running = true;
        assert!(!record.source_should_run(&boot, Some(&replacement)).unwrap());
        std::mem::swap(&mut record.intent.source, &mut record.intent.destination);
        record.intent.system_startup = Startup::Runtime;
        assert!(record.source_should_run(&boot, None).unwrap());
        assert_eq!(
            record.restoration_policy(&boot, None).unwrap().system,
            Startup::Runtime
        );
        assert!(!record.source_should_run("later-boot", None).unwrap());
    }

    #[test]
    fn old_runtime_identity_is_optional_to_read_but_required_for_ambiguous_user_recovery() {
        let root = tempfile::tempdir().unwrap();
        let record = Record::new(intent(root.path())).unwrap();
        let mut value = serde_json::to_value(&record).unwrap();
        value["intent"]
            .as_object_mut()
            .unwrap()
            .remove("user_runtime");
        let old: Record = serde_json::from_value(value).unwrap();
        old.validate().unwrap();
        assert!(old.intent.user_runtime.is_none());
        let boot = old.boot_id.as_deref().unwrap();
        assert!(old.source_should_run(boot, None).is_err());
        assert!(old.restoration_policy(boot, None).is_err());
        assert_eq!(
            old.restoration_policy("later-boot", None).unwrap().user,
            Startup::Disabled
        );
        assert!(!old.source_should_run("later-boot", None).unwrap());
    }

    #[test]
    fn checkpoints_survive_reopening_and_completion_retains_only_one_history_record() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal");
        let mut record = Record::new(intent(root.path())).unwrap();
        Store::open(&path, false).unwrap().write(&record).unwrap();
        for phase in [
            Phase::Ready,
            Phase::StoppingSource,
            Phase::SourceStopped,
            Phase::Published,
            Phase::SelectingDestination,
            Phase::StartingDestination,
            Phase::VerifyingDestination,
        ] {
            let store = Store::open(&path, false).unwrap();
            assert_eq!(store.read().unwrap().unwrap(), record);
            record = record.advance(phase).unwrap();
            store.write(&record).unwrap();
        }
        assert!(record.advance(Phase::Complete).is_err());
        record.destination_instance = Some("newly-verified-instance".into());
        record = record.advance(Phase::Complete).unwrap();
        let store = Store::open(&path, false).unwrap();
        store.write(&record).unwrap();
        store.archive().unwrap();
        assert!(store.read().unwrap().is_none());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
        assert_eq!(fs::metadata(path.join(LAST)).unwrap().mode() & 0o777, 0o600);
        let archived: Record = serde_json::from_slice(&fs::read(path.join(LAST)).unwrap()).unwrap();
        assert_eq!(archived, record);
        assert!(record.advance(Phase::StoppingDestination).is_err());
        let mut second = Record::new(intent(root.path()))
            .unwrap()
            .advance(Phase::RolledBack)
            .unwrap();
        second.intent.id = "1".repeat(32);
        store.write(&second).unwrap();
        store.archive().unwrap();
        assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
        assert_eq!(
            serde_json::from_slice::<Record>(&fs::read(path.join(LAST)).unwrap()).unwrap(),
            second
        );
    }

    #[test]
    fn carry_requires_validated_preparation_and_a_backup_before_startup() {
        let root = tempfile::tempdir().unwrap();
        let (source, target, mut prepared) = crate::state_transfer::prepared_fixture();
        prepared.destination_uid = 1000;
        let mut intent = intent(root.path());
        intent.carry_settings = true;
        intent.source = ServiceSelection {
            scope: ServiceScope::System,
            uid: prepared.destination_uid + 1,
        };
        intent.destination = ServiceSelection {
            scope: ServiceScope::User,
            uid: prepared.destination_uid,
        };
        intent.source_config = source.path().join("config.json");
        intent.destination_config = target.path().join("config.json");
        let mut record = Record::new(intent).unwrap();
        assert!(record.advance(Phase::Ready).is_err());
        assert!(record.advance(Phase::StartingDestination).is_err());
        record.prepared = Some(prepared.clone());
        for phase in [
            Phase::Ready,
            Phase::StoppingSource,
            Phase::SourceStopped,
            Phase::Publishing,
        ] {
            record = record.advance(phase).unwrap();
        }
        assert!(record.advance(Phase::Published).is_err());
        record.destination_backup = Some(".lianli-state-backup-abcdef".into());
        record = record.advance(Phase::Published).unwrap();
        record.prepared.as_mut().unwrap().decode_validated = false;
        assert!(record.validate().is_err());
        record.prepared = Some(prepared);
        record.prepared.as_mut().unwrap().id = "f".repeat(32);
        assert!(record.validate().is_err());
    }

    #[test]
    fn failed_switches_keep_original_startup_choices_through_recovery_and_rollback() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("journal"), false).unwrap();
        let initial = intent(root.path());
        let mut record = Record::new(initial.clone()).unwrap();
        for phase in [
            Phase::Ready,
            Phase::StoppingSource,
            Phase::SourceStopped,
            Phase::Published,
            Phase::SelectingDestination,
            Phase::StartingDestination,
            Phase::RecoveryRequired,
        ] {
            record = record.advance(phase).unwrap();
        }
        record.failure = Some("destination startup failed".into());
        store.write(&record).unwrap();
        let mut recovered = store.read().unwrap().unwrap();
        for phase in [
            Phase::StoppingDestination,
            Phase::RestoringDestination,
            Phase::RestoringStartup,
            Phase::StartingSource,
            Phase::VerifyingSource,
            Phase::RolledBack,
        ] {
            recovered = recovered.advance(phase).unwrap();
            store.write(&recovered).unwrap();
        }
        assert_eq!(recovered.intent, initial);
        assert_eq!(
            recovered.failure.as_deref(),
            Some("destination startup failed")
        );
        assert!(recovered.advance(Phase::StartingDestination).is_err());
    }

    #[test]
    fn corrupt_oversized_and_inconsistent_journals_are_preserved_for_recovery() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal");
        let store = Store::open(&path, false).unwrap();
        let record = Record::new(intent(root.path())).unwrap();
        store.write(&record).unwrap();
        let file = path.join(ACTIVE);
        for bytes in [b"broken JSON".to_vec(), vec![b' '; MAX_BYTES + 1]] {
            fs::write(&file, &bytes).unwrap();
            assert!(store.read().is_err());
            assert_eq!(fs::read(&file).unwrap(), bytes);
        }
        let mut invalid = record.clone();
        invalid.phase = Phase::Complete;
        fs::write(&file, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(store.read().is_err());
        invalid = record.clone();
        invalid.intent.destination = invalid.intent.source;
        assert!(invalid.validate().is_err());
        invalid = record;
        invalid.destination_backup = Some("../another-file".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn replaced_directories_and_linked_records_cannot_redirect_recovery_writes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal");
        let store = Store::open(&path, false).unwrap();
        let record = Record::new(intent(root.path())).unwrap();
        store.write(&record).unwrap();
        let hardlink = root.path().join("linked-record");
        fs::hard_link(path.join(ACTIVE), &hardlink).unwrap();
        assert!(store.read().is_err());
        fs::remove_file(hardlink).unwrap();
        let moved = root.path().join("moved");
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.write(&record).is_err());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        fs::remove_dir(&path).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        assert!(Store::open(&path, false).is_err());
        assert!(moved.join(ACTIVE).is_file());
    }
}

use crate::account::Account;
use crate::services::Route;
use crate::state::{StateSnapshot, StateSummary};
use anyhow::{ensure, Context, Result};
use lianli_shared::media_dependencies::AssetAccessReport;
use lianli_shared::services::ServiceScope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

const USER_RECIPE: &[u8] = include_bytes!("../../../packaging/systemd/lianli-daemon.service");
const SYSTEM_RECIPE: &[u8] =
    include_bytes!("../../../packaging/systemd/lianli-daemon-system.service");

#[derive(Debug, Serialize, Deserialize)]
pub struct Destination {
    pub scope: ServiceScope,
    pub uid: u32,
    pub gid: u32,
    pub groups_fingerprint: String,
    pub mount_namespace: lianli_shared::daemon::FileIdentity,
    #[serde(default)]
    pub state_directory: Option<lianli_shared::daemon::FileIdentity>,
    pub config_path: PathBuf,
    pub working_directory: PathBuf,
    pub state: Option<StateSummary>,
    pub assets: AssetAccessReport,
}

pub fn preflight(account: &Account, scope: ServiceScope) -> Result<Destination> {
    preflight_with_setup(account, scope, true)
}

pub(crate) fn preflight_existing(account: &Account, scope: ServiceScope) -> Result<Destination> {
    preflight_with_setup(account, scope, false)
}

pub(crate) fn preflight_recovery(account: &Account, config: &Path) -> Result<()> {
    let output = crate::command::run(
        account.control_command(&[
            OsStr::new("check-recovery-access"),
            OsStr::new("--config"),
            config.as_os_str(),
        ])?,
        Duration::from_secs(30),
    )?;
    ensure!(
        output.status.success(),
        "The destination home is not ready for recovery: {}",
        output.stderr.trim()
    );
    Ok(())
}

pub fn check_recovery_access(config: &Path) -> Result<()> {
    crate::container_destination::verify_config(config)?;
    ensure!(
        unsafe { libc::geteuid() } != 0 && config.is_absolute(),
        "Check recovery access under its unprivileged destination account"
    );
    let directory = crate::media_publication::Directory::open(
        config
            .parent()
            .context("Recovery configuration has no directory")?,
    )?;
    let probe = tempfile::NamedTempFile::new_in(directory.path())?;
    probe.as_file().sync_all()?;
    Ok(())
}

fn preflight_with_setup(
    account: &Account,
    scope: ServiceScope,
    prepare_directory: bool,
) -> Result<Destination> {
    if let Some(execution) = &account.container {
        ensure!(
            execution.destination.scope == scope,
            "Container destination has another service mode"
        );
        let mut args = vec![
            OsStr::new("inspect-destination"),
            OsStr::new("--scope"),
            OsStr::new(match scope {
                ServiceScope::User => "user",
                ServiceScope::System => "system",
            }),
        ];
        if prepare_directory {
            args.push(OsStr::new("--prepare-directory"));
        }
        let output = crate::command::run(account.control_command(&args)?, Duration::from_secs(60))?;
        ensure!(
            output.status.success(),
            "Container destination preflight failed: {}",
            output.stderr.trim()
        );
        let destination: Destination = serde_json::from_str(&output.stdout)?;
        ensure!(
            account.matches_destination(&destination) && destination.assets.uid == account.uid,
            "Container destination changed after discovery"
        );
        return Ok(destination);
    }
    ensure!(
        std::env::current_exe()?.file_name() == Some(OsStr::new("lianli-control")),
        "Run destination preflight through the installed standalone control helper"
    );
    if scope == ServiceScope::System {
        ensure!(
            namespace("self")? == namespace("1")?,
            "System destination must be inspected from the service manager's host namespace"
        );
        if prepare_directory {
            prepare_system_directory(account)?;
        }
    } else {
        verify_user_namespace(account)?;
    }
    let scope_arg = match scope {
        ServiceScope::User => "user",
        ServiceScope::System => "system",
    };
    let mut args = vec![
        OsStr::new("inspect-destination"),
        OsStr::new("--scope"),
        OsStr::new(scope_arg),
    ];
    if prepare_directory {
        args.push(OsStr::new("--prepare-directory"));
    }
    let command = account.control_command(&args)?;
    let result = crate::command::run(command, Duration::from_secs(30))?;
    ensure!(
        result.status.success(),
        "Destination preflight failed: {}",
        result.stderr.trim()
    );
    let report: Destination =
        serde_json::from_str(&result.stdout).context("Invalid destination preflight response")?;
    ensure!(
        report.scope == scope
            && report.uid == account.uid
            && report.gid == account.gid
            && report.groups_fingerprint == account.group_fingerprint()
            && report.mount_namespace == namespace("self")?,
        "Destination preflight ran under a different account"
    );
    ensure!(
        report.assets.uid == account.uid,
        "Destination assets were checked under a different account"
    );
    if scope == ServiceScope::User {
        verify_user_namespace(account)?;
    }
    Ok(report)
}

pub fn inspect(scope: ServiceScope, prepare_directory: bool) -> Result<Destination> {
    if let Some(destination) = crate::container_destination::current() {
        ensure!(
            destination.scope == scope,
            "Container worker was prepared for another service mode"
        );
        let account = destination.verify()?;
        return inspect_container_files(
            scope,
            &account,
            &destination.config_path,
            &destination.working_directory,
            prepare_directory,
        );
    }
    let uid = unsafe { libc::geteuid() };
    let account = match scope {
        ServiceScope::User => Account::user(uid)?,
        ServiceScope::System => Account::system()?,
    };
    account.verify_current()?;
    verify_service_recipe(scope)?;
    verify_daemon_binary()?;
    let working_directory = match scope {
        ServiceScope::User => account.home.clone(),
        ServiceScope::System => PathBuf::from("/"),
    };
    let config_path = match scope {
        ServiceScope::System => PathBuf::from("/var/lib/lianli/config.json"),
        ServiceScope::User => user_config(&user_environment()?, &working_directory)?,
    };
    if scope == ServiceScope::User && prepare_directory {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(
            config_path
                .parent()
                .context("Destination has no configuration directory")?,
        )?;
    }
    inspect_files(
        scope,
        &account,
        config_path,
        working_directory,
        prepare_directory,
    )
}

pub fn inspect_container(
    name: &str,
    scope: ServiceScope,
    config: &Path,
    working: &Path,
    prepare: bool,
) -> Result<Destination> {
    let account = crate::container_destination::current_account(name)?;
    inspect_container_files(scope, &account, config, working, prepare)
}

fn inspect_container_files(
    scope: ServiceScope,
    account: &Account,
    config: &Path,
    working: &Path,
    prepare: bool,
) -> Result<Destination> {
    crate::container_destination::validate_config(config)?;
    crate::container_destination::absolute(working)?;
    if prepare {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(config.parent().context("Configuration has no directory")?)?;
    }
    inspect_files(scope, account, config.into(), working.into(), prepare)
}

pub(crate) fn verify_service_recipe(scope: ServiceScope) -> Result<()> {
    let properties = Route::Native.output(
        "/usr/bin/systemctl",
        &[
            scope.argument(),
            "--no-pager",
            "--no-ask-password",
            "show",
            "--all",
            "--property=LoadState,FragmentPath,DropInPaths,NeedDaemonReload,WorkingDirectory",
            scope.unit(),
        ],
    )?;
    ensure!(
        properties.status.success(),
        "Cannot inspect destination unit: {}",
        properties.stderr.trim()
    );
    let properties: HashMap<_, _> = properties
        .stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let fragment = verify_properties(&properties, scope)?;
    let default_working = match scope {
        ServiceScope::User => Account::user(unsafe { libc::geteuid() })?.home,
        ServiceScope::System => PathBuf::from("/"),
    };
    verify_working_directory(
        properties.get("WorkingDirectory").copied().unwrap_or(""),
        &default_working,
    )?;
    verify_recipe(&fragment, scope)?;
    if properties
        .get("DropInPaths")
        .is_some_and(|paths| !paths.is_empty())
    {
        verify_drop_ins(scope)?;
    }
    Ok(())
}

fn verify_working_directory(value: &str, expected: &Path) -> Result<()> {
    ensure!(
        value.is_empty() || Path::new(value.strip_prefix('!').unwrap_or(value)) == expected,
        "Service working directory must be {} for automatic switching",
        expected.display()
    );
    Ok(())
}

fn verify_drop_ins(scope: ServiceScope) -> Result<()> {
    #[derive(Deserialize)]
    struct Paths {
        data: Vec<String>,
    }
    let object = format!(
        "/org/freedesktop/systemd1/unit/{}",
        scope.unit().replace('-', "_2d").replace('.', "_2e")
    );
    let output = Route::Native.output(
        "/usr/bin/busctl",
        &[
            scope.argument(),
            "--timeout=4",
            "--json=short",
            "get-property",
            "org.freedesktop.systemd1",
            &object,
            "org.freedesktop.systemd1.Unit",
            "DropInPaths",
        ],
    )?;
    ensure!(
        output.status.success(),
        "Cannot inspect service overrides: {}",
        output.stderr.trim()
    );
    let paths: Paths = serde_json::from_str(&output.stdout)?;
    ensure!(
        paths.data.len() <= 32,
        "Too many service overrides to inspect"
    );
    for path in paths.data {
        let owners = if scope == ServiceScope::User {
            vec![0, unsafe { libc::geteuid() }]
        } else {
            vec![0]
        };
        let resolved = crate::protected_path::resolve(Path::new(&path), Path::new("/"), &owners)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(resolved)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.mode() & 0o022 == 0
                && (metadata.uid() == 0
                    || scope == ServiceScope::User && metadata.uid() == unsafe { libc::geteuid() }),
            "Service override has unexpected ownership or write permissions: {path}"
        );
        let mut bytes = Vec::new();
        file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 16 * 1024,
            "Service override is too large: {path}"
        );
        verify_override(std::str::from_utf8(&bytes)?)
            .with_context(|| format!("Unsupported service override {path}"))?;
    }
    Ok(())
}

fn verify_override(text: &str) -> Result<()> {
    let mut service = false;
    let mut install = false;
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(['#', ';']))
    {
        if line.starts_with('[') {
            service = line == "[Service]";
            install = line == "[Install]";
            continue;
        }
        let (key, value) = line.split_once('=').context("Invalid override directive")?;
        if install && key.trim() == "WantedBy" {
            ensure!(
                value
                    .split_whitespace()
                    .all(|target| matches!(target, "default.target" | "multi-user.target")),
                "Unsupported service installation target"
            );
            continue;
        }
        if service
            && matches!(
                key.trim(),
                "KillMode"
                    | "SendSIGKILL"
                    | "KillSignal"
                    | "RestartKillSignal"
                    | "FinalKillSignal"
                    | "TimeoutStopFailureMode"
                    | "TimeoutStopSec"
                    | "TimeoutAbortSec"
            )
        {
            continue;
        }
        ensure!(
            service && key.trim() == "Environment",
            "Override directive {} affects the verified service recipe",
            key.trim()
        );
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if value.len() >= 2
            && (value.starts_with('"') && value.ends_with('"')
                || value.starts_with('\'') && value.ends_with('\''))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        ensure!(
            !value.contains(['"', '\'', '\\']) && !value.chars().any(char::is_whitespace),
            "Use one simple environment assignment per override line"
        );
        let (name, _) = value
            .split_once('=')
            .context("Invalid environment assignment")?;
        ensure!(
            matches!(
                name,
                "LIANLI_ENABLE_HW_VIDEO"
                    | "PATH"
                    | "LOCALE_ARCHIVE"
                    | "TZDIR"
                    | "RUST_LOG"
                    | "DISPLAY"
                    | "WAYLAND_DISPLAY"
                    | "HYPRLAND_INSTANCE_SIGNATURE"
            ),
            "Service override {name} is unsupported for automatic switching"
        );
    }
    Ok(())
}

fn inspect_files(
    scope: ServiceScope,
    account: &Account,
    config_path: PathBuf,
    working_directory: PathBuf,
    prepare: bool,
) -> Result<Destination> {
    let uid = account.uid;
    let base = config_path
        .parent()
        .context("Destination configuration has no directory")?;
    let directory = OpenOptions::new().read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(base)
        .context("Destination configuration directory is unavailable. Complete account setup before switching")?;
    let mut metadata = directory.metadata()?;
    // The daemon creates this directory under the session umask, which may allow group writes.
    if prepare && metadata.uid() == uid && metadata.mode() & 0o022 != 0 {
        directory.set_permissions(fs::Permissions::from_mode(metadata.mode() & 0o7755))?;
        metadata = directory.metadata()?;
    }
    ensure!(
        metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "State directory {} must belong to its daemon account and must not be writable by other accounts",
        base.display()
    );
    ensure!(!entry_present(&base.join(".lianli-state-transaction.json"))?,
        "Destination has an interrupted state transaction. Recover it before preparing another switch");
    let writable =
        tempfile::NamedTempFile::new_in(format!("/proc/self/fd/{}", directory.as_raw_fd()))
            .context("Destination account cannot write its configuration directory")?;
    writable.as_file().sync_all()?;
    drop(writable);
    let snapshot = match fs::symlink_metadata(&config_path) {
        Ok(_) => Some(StateSnapshot::read(&config_path, &working_directory)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Inspecting destination configuration"),
    };
    let assets = if let Some(snapshot) = &snapshot {
        ensure!(
            snapshot.summary.issue_count == 0,
            "Destination state has invalid settings: {}",
            snapshot.summary.issues.join("\n")
        );
        snapshot.check_assets(&crate::media_staging::CopyControl::new(
            Duration::from_secs(20),
        ))?
    } else {
        for name in ["lcd_templates.json", "rgb_presets.json", "profiles"] {
            ensure!(!entry_present(&base.join(name))?, "Destination has saved auxiliary state without config.json. Repair it before switching");
        }
        AssetAccessReport {
            uid,
            checked: 0,
            failed: 0,
            issues: Vec::new(),
        }
    };
    let current = fs::symlink_metadata(base)?;
    ensure!(
        current.is_dir() && current.dev() == metadata.dev() && current.ino() == metadata.ino(),
        "Destination state directory changed during preflight"
    );
    Ok(Destination {
        scope,
        uid,
        gid: account.gid,
        groups_fingerprint: account.group_fingerprint(),
        mount_namespace: namespace("self")?,
        state_directory: Some(lianli_shared::daemon::FileIdentity {
            device: metadata.dev().to_string(),
            inode: metadata.ino().to_string(),
        }),
        config_path,
        working_directory,
        assets,
        state: snapshot.map(|value| value.summary),
    })
}

fn entry_present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("Inspecting destination state entry"),
    }
}

fn prepare_system_directory(account: &Account) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && account.uid == Account::system()?.uid,
        "System destination setup requires its authorized native administrator"
    );
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/var/lib")?;
    let metadata = parent.metadata()?;
    ensure!(
        metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "System state parent is not protected and root-owned"
    );
    let parent_path = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    let path = parent_path.join("lianli");
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        let temporary = tempfile::Builder::new()
            .prefix(".lianli-setup-")
            .tempdir_in(&parent_path)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(temporary.path())?;
        ensure!(
            unsafe { libc::fchown(directory.as_raw_fd(), account.uid, account.gid) } == 0,
            "Cannot assign destination state directory: {}",
            std::io::Error::last_os_error()
        );
        directory.set_permissions(fs::Permissions::from_mode(0o750))?;
        directory.sync_all()?;
        let name =
            std::ffi::CString::new(temporary.path().file_name().unwrap().as_encoded_bytes())?;
        let result = unsafe {
            libc::renameat2(
                parent.as_raw_fd(),
                name.as_ptr(),
                parent.as_raw_fd(),
                c"lianli".as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            let _ = temporary.keep();
            parent.sync_all()?;
        } else {
            let error = std::io::Error::last_os_error();
            ensure!(
                error.kind() == std::io::ErrorKind::AlreadyExists,
                "Creating system state directory: {error}"
            );
        }
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = directory.metadata()?;
    ensure!(metadata.uid() == account.uid && metadata.gid() == account.gid && metadata.mode() & 0o022 == 0,
        "Existing system state directory has the wrong owner, group or write permissions. Repair it before switching");
    Ok(())
}

fn namespace(pid: &str) -> Result<lianli_shared::daemon::FileIdentity> {
    let metadata = fs::metadata(format!("/proc/{pid}/ns/mnt"))
        .with_context(|| format!("Cannot verify mount namespace for process {pid}"))?;
    Ok(lianli_shared::daemon::FileIdentity {
        device: metadata.dev().to_string(),
        inode: metadata.ino().to_string(),
    })
}

fn verify_user_namespace(account: &Account) -> Result<()> {
    // Looking up a bus owner PID does not activate a lazily connected manager.
    let activate = [
        "--user",
        "--timeout=4",
        "get-property",
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "Version",
    ];
    let activate: Vec<_> = activate.iter().map(OsStr::new).collect();
    let output = crate::command::run(
        account.command("/usr/bin/busctl", &activate)?,
        Duration::from_secs(6),
    )?;
    ensure!(
        output.status.success(),
        "The user service manager is unavailable: {}",
        output.stderr.trim()
    );
    #[derive(Deserialize)]
    struct Pid {
        #[serde(rename = "type")]
        signature: String,
        data: Vec<u32>,
    }
    let arguments = [
        "--user",
        "--timeout=4",
        "--json=short",
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        "s",
        "org.freedesktop.systemd1",
    ];
    let arguments: Vec<_> = arguments.iter().map(OsStr::new).collect();
    let output = crate::command::run(
        account.command("/usr/bin/busctl", &arguments)?,
        Duration::from_secs(6),
    )?;
    ensure!(
        output.status.success(),
        "Cannot verify the user manager's namespace: {}",
        output.stderr.trim()
    );
    let pid: Pid = serde_json::from_str(&output.stdout)?;
    ensure!(
        pid.signature == "u" && pid.data.len() == 1 && pid.data[0] != 0,
        "Invalid user manager process identity"
    );
    ensure!(namespace(&pid.data[0].to_string())? == namespace("self")?,
        "The user service uses another mount namespace. Automatic native switching cannot validate its files from this helper");
    let file = fs::File::open(format!("/proc/{}/status", pid.data[0]))?;
    let mut status = String::new();
    file.take(64 * 1024 + 1).read_to_string(&mut status)?;
    ensure!(
        status.len() <= 64 * 1024,
        "User manager credentials exceed their read limit"
    );
    verify_manager_groups(&status, account)?;
    Ok(())
}

fn verify_manager_groups(status: &str, account: &Account) -> Result<()> {
    let fields: HashMap<_, _> = status
        .lines()
        .filter_map(|line| line.split_once(':'))
        .collect();
    let numbers = |key| -> Result<Vec<u32>> {
        fields
            .get(key)
            .context("User manager credentials are unavailable")?
            .split_whitespace()
            .map(|value| value.parse().map_err(Into::into))
            .collect()
    };
    let uid = numbers("Uid")?;
    let gid = numbers("Gid")?;
    ensure!(
        uid.len() == 4
            && gid.len() == 4
            && uid.iter().all(|uid| *uid == account.uid)
            && gid.iter().all(|gid| *gid == account.gid),
        "User manager belongs to another account or primary group"
    );
    let mut groups = numbers("Groups")?;
    groups.push(account.gid);
    groups.sort_unstable();
    groups.dedup();
    ensure!(groups == account.groups,
        "The user service manager has stale supplementary groups. Log out completely and log in again, or reboot if user lingering keeps the manager running");
    Ok(())
}

fn verify_properties(properties: &HashMap<&str, &str>, scope: ServiceScope) -> Result<PathBuf> {
    ensure!(
        properties.get("LoadState") == Some(&"loaded")
            && properties.get("NeedDaemonReload") == Some(&"no"),
        "The service unit must be loaded and current. Run daemon-reload after updating it"
    );
    let path = PathBuf::from(
        properties
            .get("FragmentPath")
            .context("Destination unit path is unverified")?,
    );
    let directory = match scope {
        ServiceScope::User => "user",
        ServiceScope::System => "system",
    };
    ensure!(
        ["/usr/lib/systemd", "/lib/systemd", "/etc/systemd"]
            .into_iter()
            .any(|prefix| path == Path::new(prefix).join(directory).join(scope.unit())),
        "Automatic switching requires an installed native service unit"
    );
    Ok(path)
}

fn verify_recipe(path: &Path, scope: ServiceScope) -> Result<()> {
    let path = crate::protected_path::resolve(path, Path::new("/"), &[0])?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
            && metadata.len() <= 16 * 1024,
        "Native destination unit must be a protected root-owned file"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes == match scope { ServiceScope::User => USER_RECIPE, ServiceScope::System => SYSTEM_RECIPE },
        "Install the current supplied native unit and reload the manager before automatic switching");
    Ok(())
}

pub(crate) fn verify_daemon_binary() -> Result<()> {
    let path =
        crate::protected_path::resolve(Path::new("/usr/bin/lianli-daemon"), Path::new("/"), &[0])?;
    let binary = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .context("Installed daemon is unavailable")?;
    let metadata = binary.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
            && metadata.mode() & 0o111 != 0,
        "Install a protected root-owned daemon binary before automatic switching"
    );
    let output = Route::Native.output(
        path.to_str()
            .context("Installed daemon path is not UTF-8")?,
        &["capabilities"],
    )?;
    ensure!(output.status.success(), "Installed daemon cannot report its capabilities: {}. Update it and its runtime libraries before switching", output.stderr.trim());
    verify_build(
        &serde_json::from_str(&output.stdout)
            .context("Invalid installed daemon capability report")?,
    )
}

fn verify_build(build: &lianli_shared::daemon::DaemonBuildInfo) -> Result<()> {
    use lianli_shared::daemon::*;
    ensure!(
        build.version == env!("CARGO_PKG_VERSION")
            && build.protocol_version == IPC_PROTOCOL_VERSION,
        "Install matching control and daemon versions before switching"
    );
    ensure!([GUARDED_WRITES, GRACEFUL_SHUTDOWN, SERVICE_WRITE_GATE, SERVICE_SELECTION, SERVICE_STARTUP_GATE, MEDIA_DECODE]
        .iter().all(|required| build.capabilities.iter().any(|value| value == required)),
        "Installed daemon lacks the shutdown, write coordination, host selection, startup gate or media validation support needed for switching");
    Ok(())
}

fn user_environment() -> Result<HashMap<String, String>> {
    #[derive(Deserialize)]
    struct Environment {
        #[serde(rename = "type")]
        signature: String,
        data: Vec<String>,
    }
    let output = Route::Native.output(
        "/usr/bin/busctl",
        &[
            "--user",
            "--timeout=4",
            "--json=short",
            "get-property",
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "Environment",
        ],
    )?;
    ensure!(
        output.status.success(),
        "Cannot read the user service environment: {}",
        output.stderr.trim()
    );
    let environment: Environment = serde_json::from_str(&output.stdout)?;
    ensure!(
        environment.signature == "as",
        "Unexpected user manager environment format"
    );
    let mut values = HashMap::new();
    for value in environment.data {
        if let Some((key, value)) = value.split_once('=') {
            if matches!(key, "HOME" | "XDG_CONFIG_HOME") {
                values.insert(key.to_string(), value.to_string());
            }
        }
    }
    Ok(values)
}

fn user_config(environment: &HashMap<String, String>, working: &Path) -> Result<PathBuf> {
    let home = environment
        .get("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| working.to_path_buf());
    let base = environment
        .get("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let path = base.join("lianli/config.json");
    let path = if path.is_absolute() {
        path
    } else {
        working.join(path)
    };
    ensure!(
        path.as_os_str().len() <= 4096,
        "Destination configuration path exceeds its limit"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_daemon_requires_matching_protocol_and_safe_switching_capabilities() {
        use lianli_shared::daemon::*;
        let mut build = DaemonBuildInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: IPC_PROTOCOL_VERSION,
            capabilities: vec![
                GUARDED_WRITES.into(),
                GRACEFUL_SHUTDOWN.into(),
                SERVICE_WRITE_GATE.into(),
                SERVICE_SELECTION.into(),
                SERVICE_STARTUP_GATE.into(),
                MEDIA_DECODE.into(),
            ],
        };
        assert!(verify_build(&build).is_ok());
        build.capabilities.pop();
        assert!(verify_build(&build).is_err());
        build.capabilities.push(MEDIA_DECODE.into());
        build
            .capabilities
            .retain(|value| value != SERVICE_STARTUP_GATE);
        assert!(verify_build(&build).is_err());
        build.capabilities.push(SERVICE_STARTUP_GATE.into());
        assert!(verify_build(&build).is_ok());
        build.protocol_version += 1;
        assert!(verify_build(&build).is_err());
        build.protocol_version = IPC_PROTOCOL_VERSION;
        build.version = "old".into();
        assert!(verify_build(&build).is_err());
    }

    #[test]
    fn stale_manager_credentials_cannot_validate_destination_access() {
        let account = Account {
            uid: 1000,
            container: None,
            gid: 100,
            groups: vec![100, 200],
            name: "fixture".into(),
            home: "/home/fixture".into(),
        };
        assert!(verify_manager_groups(
            "Uid:\t1000 1000 1000 1000\nGid:\t100 100 100 100\nGroups:\t200\n",
            &account
        )
        .is_ok());
        for status in [
            "Uid: 1000 1000 1000 1000\nGid: 100 100 100 100\nGroups: 100\n",
            "Uid: 1001 1001 1001 1001\nGid: 100 100 100 100\nGroups: 100 200\n",
            "Uid: 1000 1000 1000 1000\nGid: 300 300 300 300\nGroups: 100 200\n",
            "Groups: 100 200\n",
        ] {
            assert!(verify_manager_groups(status, &account).is_err());
        }
    }

    fn account(home: &Path) -> Account {
        Account {
            uid: unsafe { libc::geteuid() },
            container: None,
            gid: unsafe { libc::getegid() },
            groups: vec![],
            name: "fixture".into(),
            home: home.into(),
        }
    }

    #[test]
    fn fresh_destination_is_writable_without_publishing_a_config_and_orphans_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let account = account(root.path());
        let config = root.path().join("config.json");
        let check = || {
            inspect_files(
                ServiceScope::User,
                &account,
                config.clone(),
                root.path().into(),
                false,
            )
        };
        let report = check().unwrap();
        assert!(report.state.is_none());
        assert_eq!(report.assets.checked, 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        std::os::unix::fs::symlink("missing", root.path().join("profiles")).unwrap();
        assert!(check().is_err());
        fs::remove_file(root.path().join("profiles")).unwrap();
        fs::write(root.path().join(".lianli-state-transaction.json"), b"{}").unwrap();
        assert!(check().is_err());
    }

    #[test]
    fn preparation_removes_group_write_left_by_the_session_umask() {
        let root = tempfile::tempdir().unwrap();
        let account = account(root.path());
        let config = root.path().join("config.json");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o775)).unwrap();
        let check = |prepare| {
            inspect_files(
                ServiceScope::User,
                &account,
                config.clone(),
                root.path().into(),
                prepare,
            )
        };
        let error = check(false).unwrap_err().to_string();
        assert!(error.contains(&root.path().display().to_string()));
        assert_eq!(fs::metadata(root.path()).unwrap().mode() & 0o777, 0o775);
        check(true).unwrap();
        assert_eq!(fs::metadata(root.path()).unwrap().mode() & 0o777, 0o755);
        check(false).unwrap();
    }

    #[test]
    fn destination_preflight_includes_inactive_media_and_retains_original_files() {
        let root = tempfile::tempdir().unwrap();
        let account = account(root.path());
        let config = root.path().join("config.json");
        fs::write(&config, b"{}").unwrap();
        fs::create_dir(root.path().join("profiles")).unwrap();
        let profile = root.path().join("profiles/Inactive.json");
        let bytes = br#"{"name":"Inactive","device_id":"offline","lcds":[{"index":0,"type":"video","path":"missing.mp4"}]}"#;
        fs::write(&profile, bytes).unwrap();
        let check = || {
            inspect_files(
                ServiceScope::User,
                &account,
                config.clone(),
                root.path().into(),
                false,
            )
        };
        let report = check().unwrap();
        assert_eq!(report.assets.failed, 1);
        assert!(report.assets.issues[0].owner.contains("Inactive.json"));
        assert!(report.assets.issues[0]
            .path
            .as_ref()
            .unwrap()
            .ends_with("missing.mp4"));
        fs::write(root.path().join("missing.mp4"), b"readable").unwrap();
        let report = check().unwrap();
        assert_eq!(report.assets.checked, 1);
        assert_eq!(report.state.unwrap().profiles, 1);
        assert_eq!(fs::read(&profile).unwrap(), bytes);
        assert_eq!(fs::read(&config).unwrap(), b"{}");
    }

    #[test]
    fn manager_environment_paths_are_preserved_without_shell_interpretation() {
        let home = Path::new("/home/fixture");
        let mut env = HashMap::new();
        assert_eq!(
            user_config(&env, home).unwrap(),
            home.join(".config/lianli/config.json")
        );
        env.insert("HOME".into(), "/different home".into());
        assert_eq!(
            user_config(&env, home).unwrap(),
            Path::new("/different home/.config/lianli/config.json")
        );
        env.insert("XDG_CONFIG_HOME".into(), "relative $(literal)".into());
        assert_eq!(
            user_config(&env, home).unwrap(),
            home.join("relative $(literal)/lianli/config.json")
        );
        env.insert("XDG_CONFIG_HOME".into(), "/absolute custom".into());
        assert_eq!(
            user_config(&env, home).unwrap(),
            Path::new("/absolute custom/lianli/config.json")
        );
    }

    #[test]
    fn overridden_stale_or_uninstalled_native_units_are_not_assumed_safe() {
        let mut properties = HashMap::from([
            ("LoadState", "loaded"),
            ("NeedDaemonReload", "no"),
            ("DropInPaths", ""),
            ("WorkingDirectory", ""),
            (
                "FragmentPath",
                "/usr/lib/systemd/user/lianli-daemon.service",
            ),
        ]);
        assert!(verify_properties(&properties, ServiceScope::User).is_ok());
        for (key, value) in [
            ("NeedDaemonReload", "yes"),
            (
                "FragmentPath",
                "/home/user/.config/systemd/user/lianli-daemon.service",
            ),
        ] {
            let old = properties.insert(key, value).unwrap();
            assert!(verify_properties(&properties, ServiceScope::User).is_err());
            properties.insert(key, old);
        }
        assert!(verify_properties(&properties, ServiceScope::System).is_err());
    }

    #[test]
    fn harmless_environment_overrides_and_default_user_working_directory_are_supported() {
        for text in [
            "[Service]\nEnvironment=LIANLI_ENABLE_HW_VIDEO=1\n",
            "[Service]\nEnvironment=\"RUST_LOG=info\"\nEnvironment=WAYLAND_DISPLAY=wayland-1\n",
            "[Service]\nTimeoutStopFailureMode=abort\n",
            "[Service]\nEnvironment=PATH=/nix/store/fixture/bin:/usr/bin\nEnvironment=LOCALE_ARCHIVE=/nix/store/locale\nEnvironment=TZDIR=/nix/store/zoneinfo\n[Install]\nWantedBy=default.target\n",
            "[Service]\nKillMode=control-group\nSendSIGKILL=yes\nKillSignal=SIGINT\nRestartKillSignal=SIGTERM\nFinalKillSignal=SIGABRT\nTimeoutStopSec=45s\nTimeoutAbortSec=10s\n",
        ] {
            verify_override(text).unwrap();
        }
        for text in [
            "[Service]\nExecStart=/bin/false",
            "[Service]\nEnvironment=HOME=/other",
            "[Service]\nEnvironment=LD_PRELOAD=/tmp/custom.so",
            "[Service]\nEnvironment=\"RUST_LOG=info\" HOME=/other",
            "[Service]\nEnvironment=\"",
            "[Install]\nAlso=other.service\n",
            "[Install]\nWantedBy=other.service\n",
            "[Install]\nExecStart=/bin/false\n",
        ] {
            assert!(verify_override(text).is_err(), "{text}");
        }
        verify_working_directory("!/home/fixture", Path::new("/home/fixture")).unwrap();
        verify_working_directory("/", Path::new("/")).unwrap();
        assert!(verify_working_directory("/custom", Path::new("/home/fixture")).is_err());
    }
}

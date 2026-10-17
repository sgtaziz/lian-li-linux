use crate::command::run;
use anyhow::{bail, ensure, Context, Result};
use lianli_shared::installation::InstallationContext;
use lianli_shared::services::{ServiceProbe, ServiceScope};
use serde::Serialize;
use std::process::Command;
use std::time::Duration;

pub const WORKFLOW: &str = include_str!("app_update.sh");

#[derive(Serialize)]
pub struct PackageUpdate {
    pub installed: String,
    pub available: Option<String>,
}

pub fn box_name() -> Result<String> {
    let InstallationContext::Distrobox { name } = InstallationContext::detect() else {
        bail!("This update workflow requires a Fedora Distrobox installation")
    };
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            && name.as_bytes()[0].is_ascii_alphanumeric(),
        "Invalid Distrobox name"
    );
    let release = std::fs::read_to_string("/etc/os-release")?;
    ensure!(
        release
            .lines()
            .any(|line| matches!(line, "ID=fedora" | "ID=\"fedora\"")),
        "This update workflow currently supports Fedora Distrobox only"
    );
    Ok(name)
}

pub fn verify() -> Result<()> {
    let name = box_name()?;
    let context = InstallationContext::detect();
    let report = crate::services::inspect(&context);
    let ServiceProbe::Known { value: user } = report.user else {
        bail!("Cannot verify the user daemon service")
    };
    ensure!(
        user.active_state == "active"
            && user.graceful_shutdown == Some(true)
            && user.distrobox_name.as_deref() == Some(name.as_str()),
        "An active, verified user daemon service in this box is required"
    );
    let ServiceProbe::Known { value: system } = report.system else {
        bail!("Cannot verify the system daemon service")
    };
    ensure!(
        matches!(system.active_state.as_str(), "inactive" | "failed"),
        "The system daemon service must be stopped"
    );
    let Some(ServiceProbe::Known {
        value: Some(selection),
    }) = report.selection
    else {
        bail!("Cannot verify the selected service mode")
    };
    ensure!(
        selection.allows(ServiceScope::User, unsafe { libc::geteuid() }),
        "Select user mode before updating"
    );
    let Some(ServiceProbe::Known { value: ownership }) = report.ownership else {
        bail!("Cannot verify hardware ownership")
    };
    let Some(ServiceProbe::Known { value: process }) = ownership.process else {
        bail!("Cannot verify the hardware owner's service")
    };
    ensure!(
        ownership.owner_pid == Some(process.pid)
            && process.service == Some(ServiceScope::User)
            && process.effective_uid == unsafe { libc::geteuid() },
        "Hardware ownership does not match the user daemon service"
    );
    Ok(())
}

pub fn check() -> Result<PackageUpdate> {
    box_name()?;
    let mut rpm = Command::new("rpm");
    rpm.args(["-q", "--qf", "%{VERSION}-%{RELEASE}", "lian-li-linux"]);
    let installed = run(rpm, Duration::from_secs(10))?;
    ensure!(
        installed.status.success(),
        "lian-li-linux must be installed as an RPM package"
    );
    let mut dnf = Command::new("dnf");
    dnf.env("LC_ALL", "C")
        .args(["-q", "--refresh", "check-upgrade", "lian-li-linux"]);
    let output = run(dnf, Duration::from_secs(120)).context("Checking package repositories")?;
    let available = candidate(output.status.code(), &output.stdout, &output.stderr)?;
    Ok(PackageUpdate {
        installed: installed.stdout.trim().into(),
        available,
    })
}

pub fn launch() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    verify()?;
    let name = box_name()?;
    let bridge = crate::services::host_spawn_bridge().context("host-spawn is required")?;
    let home = std::env::var_os("HOME").context("HOME is missing")?;
    let directory = std::path::PathBuf::from(home).join(".local/state/lianli-linux");
    std::fs::create_dir_all(&directory)?;
    let job = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(directory)?;
    let script = job.path().join("update.sh");
    std::fs::write(&script, WORKFLOW)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o600))?;
    let mut command = Command::new(bridge);
    command
        .args(["systemd-run", "--user", "--collect", "--property=Type=exec"])
        .arg(format!(
            "--unit=lianli-app-update-{}",
            job.path().file_name().unwrap().to_string_lossy()
        ));
    for key in ["DISPLAY", "WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP"] {
        if let Ok(value) = std::env::var(key) {
            command.arg(format!("--setenv={key}={value}"));
        }
    }
    command
        .args([
            "--",
            "/usr/bin/konsole",
            "--separate",
            "--nofork",
            "-e",
            "/usr/bin/bash",
        ])
        .arg(&script)
        .arg(name)
        .arg(job.path());
    let output = run(command, Duration::from_secs(15))?;
    ensure!(
        output.status.success(),
        "Could not open update terminal: {}",
        output.stderr
    );
    // The transient service owns this directory after submission, even if its
    // terminal takes longer than the handshake timeout to appear.
    let path = job.keep();
    for _ in 0..200 {
        if path.join("ready").is_file() {
            std::fs::write(path.join("proceed"), "proceed\n")?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("The update terminal did not become ready. Keep the app open and inspect the terminal.")
}

fn candidate(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Option<String>> {
    match code {
        Some(0) => Ok(None),
        Some(100) => stdout
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                fields
                    .next()?
                    .starts_with("lian-li-linux.")
                    .then(|| fields.next().map(str::to_owned))
                    .flatten()
            })
            .map(Some)
            .context("DNF reported an update without a package version"),
        _ => bail!("DNF update check failed: {}", stderr.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_updates_no_update_and_repository_failure() {
        assert_eq!(candidate(Some(0), "", "").unwrap(), None);
        assert_eq!(
            candidate(Some(100), "lian-li-linux.x86_64 1.2.2-1.fc44 copr\n", "")
                .unwrap()
                .as_deref(),
            Some("1.2.2-1.fc44")
        );
        assert!(candidate(Some(1), "", "cannot refresh metadata").is_err());
        assert!(candidate(Some(100), "unrelated.x86_64 2.0 repo", "").is_err());
    }
}

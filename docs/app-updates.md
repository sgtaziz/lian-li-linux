# Updating from Settings

The application update controls currently support an RPM installation inside a
Fedora Distrobox with a verified, active user daemon. The host must provide KDE
Konsole, systemd user services and the installed host-spawn bridge. Native
installations, system daemon mode and other container distributions are not
supported by this workflow.

**Check for updates** refreshes the configured DNF repositories and checks only
the `lian-li-linux` package. It reports the installed and available RPM versions;
repository failures are shown as errors rather than “no updates”.

Save settings and close the template editor before choosing **Update in terminal**.
After confirmation, a separate host terminal is started by the systemd user
manager. The GUI closes only after the terminal signals that it is ready.
The terminal waits for all of the user's Lian Li GUI windows to close, verifies
the service installation again, and requests administrator credentials on both
the host and inside the box.

The workflow stops the active capture helper and daemon, reserves shared hardware
ownership, backs up the user configuration, and runs:

```sh
sudo dnf upgrade --refresh --setopt=install_weak_deps=False lian-li-linux
```

It refreshes the host copies of the packaged USB rules and tmpfiles configuration,
restarts the services that were stopped, and opens the updated GUI in a separate
transient unit. Backups remain under `~/.local/state/lianli-linux/update-backups/`.
DNF may ask for transaction confirmation. Keep the terminal open until completion.

On failure or cancellation, the workflow attempts to restore the stopped services
only after RPM verifies the installed package. It does not automatically downgrade
packages or roll back a partially completed host-rule installation. If package
verification or service startup fails, the terminal leaves an error for manual
recovery and does not announce success. Updates are never installed in the
background or as part of checking.

If DNF fails and services recover, the terminal asks the user to review the DNF
error, reopen the application and check for updates before trying again. If a
service fails to restart, it prints host `systemctl status` and `journalctl`
commands for that service and suggests one manual restart only after package
verification passes. Failed package verification instead asks the user to keep
the output and seek package repair before restarting. The update is never
automatically repeated after these failures.

## Development checks

```sh
bash crates/lianli-control/tests/app_update_workflow.sh
cargo test -p lianli-control app_update
cargo check -p lianli-gui
cd crates/lianli-gui
npm ci
npm run typecheck
```

The shell regression checks replace service, package-manager and host commands;
they do not stop real services or install packages. Before release, test an actual
package update in a Fedora Distrobox, including password prompts, user cancellation,
terminal handoff and service recovery.

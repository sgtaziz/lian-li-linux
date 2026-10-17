# Distrobox installation and recovery

Use a Fedora Distrobox for the existing COPR package on immutable hosts such as Bazzite.
Each section identifies whether commands run on the **host** or **inside the box**.

Installation Health labels the GUI and connected daemon separately, for example
`GUI: Native · Daemon: Distrobox (fedora)`. Each process detects its own environment.
An older or disconnected daemon shows `Daemon: Unverified`. Running the GUI on the
host does not change where the daemon runs. Host checks and service actions still
follow the GUI's installation context.

## Create a Fedora box

If you do not have Distrobox and a container engine installed, follow the
[Distrobox installation instructions](https://distrobox.it/#installation) for your host.
Use rootless Podman for this setup.

Run these commands in a **host terminal**, as your normal user:

```sh
distrobox list
distrobox create --name lianli --image registry.fedoraproject.org/fedora:44
distrobox enter --name lianli
```

If you already have a suitable Fedora box, skip creation and enter its name instead.
On hosts using NVIDIA's proprietary driver, add `--nvidia` to the create command
to enable [host driver integration](https://distrobox.it/usage/distrobox-create/#nvidia-integration).

Keep the default host home, device and process sharing. Do not use `sudo`, `--root`,
`--init` or `--unshare-*` for this setup. Both daemon service modes use host-managed
services; system mode does not require systemd running inside the container.

Wait for first-entry setup to finish, then continue below **inside the box**.
Use `exit` to return to the host for later host setup sections. Wherever this guide
asks for your box name, use `lianli` or the existing name you chose.

## Install inside the box

```sh
sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo dnf copr enable sgtaziz/lian-li-linux
sudo dnf install --setopt=install_weak_deps=False lian-li-linux
```

Keep weak dependencies disabled here. DisplayLink's DKMS build can invoke dracut, which
fails in a container. Do not enable the DisplayLink COPR inside the box. Kernel modules
belong on the host and require a method supported by that host, including its kernel updates.
Installing a module inside the box does not add it to an immutable host.

EVDI desktop mode also needs the EVDI userspace library inside the box. The package
only suggests it, so install it explicitly, then restart the daemon service:

```sh
sudo dnf install --setopt=install_weak_deps=False lian-li-linux-evdi
```

This installs only the library. The EVDI kernel module still belongs on the host.
Installation Health warns when the host has loaded EVDI but the daemon cannot load
the library.

## Desktop modules on an immutable host

Hyprland's native headless backend needs neither Hermes-KMS nor EVDI. For other
desktops, install the optional backend on the host before configuring capture
inside the box. See [desktop backends](desktop-backends.md).

On Bazzite, use the distribution's
[DisplayLink recipe](https://github.com/ublue-os/bazzite/blob/main/system_files/desktop/shared/usr/share/ublue-os/just/82-bazzite-apps.just)
when EVDI is needed. Run `ujust install-displaylink` in a **host terminal** and
follow its reboot instructions. It layers the distribution's package through
rpm-ostree. Keep the host image and its kernel-module packages updated together.
Do not copy a module built against a different kernel from the box.

Some Bazzite images have a
[known repository-override failure](https://github.com/ublue-os/bazzite/issues/4936)
in that recipe. If installation fails, use Bazzite's current support guidance.
Installing the package inside Distrobox cannot fix the host deployment.

After reboot, `modinfo -k "$(uname -r)" evdi` in a host terminal checks whether a
module is installed for the running kernel. It does not prove that the module
loads or that capture works. The box also needs the
[EVDI userspace library](#install-inside-the-box) and access to the host's DRM nodes.

For Hermes-KMS or another immutable distribution, use a host-maintained package
or image that supplies the module for each kernel update. This project does not
provide an immutable-host Hermes-KMS package. Do not treat a one-time manual
module build as an update-safe installation.

## Configure USB access and ownership on the host

Replace the value of `box_name` with your actual box name. Keep the quotes when using it:

```sh
box_name='your-box-name'
getent group lianli >/dev/null || sudo groupadd --system lianli
sudo usermod --append --groups lianli "$USER"
distrobox-enter --name "$box_name" -- cat /usr/lib/udev/rules.d/60-lianli.rules \
  | sudo tee /etc/udev/rules.d/60-lianli.rules >/dev/null
distrobox-enter --name "$box_name" -- cat /usr/lib/tmpfiles.d/lianli.conf \
  | sudo tee /etc/tmpfiles.d/lianli.conf >/dev/null
sudo udevadm control --reload-rules
sudo udevadm trigger
sudo systemd-tmpfiles --create lianli.conf
```

Log out and back in. If the box stayed running, stop it from the host with
`distrobox stop "$box_name"`, then enter it again. The running box and user service need
the updated supplementary groups; a matching group name inside the box does not prove
that numeric credentials match the host's device nodes.

Installation Health compares each checked process's numeric groups with the group
visible on the host ownership lock. It distinguishes an account database updated
after the process started from missing group setup or container mappings. Results
are labeled separately for the desktop process and connected daemon. The standalone
`lianli-control diagnose-runtime` command works inside the box without GUI libraries.
A successful lock permission check does not establish USB or compositor access.
The connected daemon also checks USB/HID nodes inside its own mount namespace.
Visible host sysfs entries with hidden guest device nodes remain unverified;
rules installed only inside the box cannot repair host permissions. Passing node
permissions does not verify container device policies or driver interface claims.

Refresh the copied host rules after package updates. They override native vendor rules
with the same filename. Review them if you later switch to a native installation.

For Fedora boxes using the user daemon and KDE Konsole, Settings provides
[update controls](app-updates.md) that coordinate the terminal package upgrade,
host rules, daemon shutdown and restart.

The daemon inside the box must see `/run/host/run/lianli-daemon.lock`. Do not create a
private container lock to bypass an integration error, or remove an active host lock.
See [ownership recovery](service-modes.md).

## Start from the host user service

A default box does not run systemd. Create `~/.config/systemd/user/lianli-daemon.service`
on the **host**, using the installed path from `command -v distrobox-enter`:

The application can generate the unit for you. Run **inside the intended box**:

```sh
lianli-control distrobox-service-unit
```

This prints a unit for the detected box. It does not write a service file, reload
systemd or start the daemon. Review the output and save it to the host path above.
For another box, pass `--box 'existing-box-name'`. For custom installations, pass
`--distrobox-enter '/absolute/host/path/distrobox-enter'` and
`--binaries '/absolute/guest/binary directory'`. The default paths are `/usr/bin`.
The generator quotes spaces and escapes literal systemd expansion characters in
guest paths while preserving the invocation variable used for graceful shutdown.
Use an existing name accepted by your container runtime.

The generated unit follows this recipe:

```ini
[Unit]
Description=Lian Li Linux Daemon (Distrobox)
After=graphical-session.target

[Service]
ExecStart=/usr/bin/env --unset=INVOCATION_ID /usr/bin/distrobox-enter --name "your-box-name" -- /usr/bin/lianli-daemon --service-invocation ${INVOCATION_ID}
ExecStop=/usr/bin/env --unset=INVOCATION_ID /usr/bin/distrobox-enter --name "your-box-name" -- /usr/bin/lianli-control stop-service --invocation-id ${INVOCATION_ID}
ExecStop=/usr/bin/sh -c 'test -z "$$1" || exec /usr/bin/timeout 5s /usr/bin/tail --pid="$$1" --sleep-interval=0.1 -f /dev/null' -- "${MAINPID}"
Restart=on-failure
RestartSec=5s
KillMode=control-group
SendSIGKILL=no
TimeoutStopSec=120s

[Install]
WantedBy=default.target
```

The guarded stop command waits for the daemon to exit. A second command waits up
to five seconds for the original Podman wrapper to finish reporting that exit.
It sends no signals and skips the wait if the wrapper has already exited.
Systemd then sends SIGTERM
to remaining helpers in this unit's control group so Podman helpers do not block
the next start. Forced SIGKILL remains disabled. Regenerate older recipes that
used `KillMode=mixed` before testing restart. The environment wrapper keeps
Podman's shared box supervisor outside this service's control group. Systemd
still supplies the invocation ID as an explicit argument to the daemon and stop
helper. Preserve both parts of the generated recipe.

Replace `your-box-name` with the box's name. Keep both commands on the same box and
use the installed absolute paths for both binaries inside it. The application binaries,
including `lianli-control`, stay installed inside the box; no host copy is required.
GUI service actions use a separate transient host user service to run that helper
inside the box, so closing the GUI does not stop service verification. The host needs
`systemd-run`; the helper and its bounded progress record stay in the box's user
runtime view. See [service operation lifetime](service-modes.md#service-operation-lifetime).
Copy `${INVOCATION_ID}` literally into the unit. Systemd supplies a new invocation ID
on each service start, and the stop command refuses a different invocation or a manual
daemon. It requests graceful shutdown over IPC and waits up to ninety seconds for the
daemon process to exit, without sending it a forced signal. This also applies to an
external `systemctl --user stop` or `restart`.

The GUI verifies the exact start/stop arguments through the host's `busctl`, then checks
the daemon's invocation and kernel lock ownership. Container IPC checks also compare
PID namespaces and process start times with the host owner. Missing host process access,
an unsupported `pidfd_open` syscall or a stale recipe prevents verified controls;
Installation Health and the command error identify the missing prerequisite. The native
daemon can still run without this wrapper integration.

Ensure no native system service or manually launched daemon is already controlling the
devices, then run **on the host**:

```sh
systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon.service
distrobox-enter --name "$box_name" -- lianli-gui
```

For this service recipe, launch the GUI in the same box to use its runtime socket
and verified Distrobox service controls.
Inspect the host journal if the GUI stays offline:

```sh
journalctl --user -u lianli-daemon.service -b -n 100 --no-pager
```

### Test a box daemon with a host GUI

A box can have a private `/run/user` directory. Its default daemon socket then
cannot be reached by a GUI running on the host. For a manual user-mode test,
stop the existing daemon first and launch the test binary **inside the box** with
the host-visible socket:

```sh
./lianli-daemon --socket "/run/host/run/user/$(id -u)/lianli-daemon.sock"
```

The host must already have selected user mode for this UID. A launch rejected by
the ownership selection is inactive and does not create a socket. Do not run a
second daemon alongside a service to work around that guard.

The host GUI can connect to this socket. Its service controls still use the native
installation context, so use the test daemon's terminal to stop it. Selected media
must be accessible at the same paths inside the box. Installation Health reports
the GUI and daemon environments separately.

## Start desktop capture at login

Desktop displays also need the logged-in user's capture helper. Generate its
separate unit **inside the box**:

```sh
lianli-control distrobox-service-unit --desktop-session
```

Review the output and save it as `~/.config/systemd/user/lianli-session.service`
on the **host**. The same `--box`, `--distrobox-enter` and `--binaries` overrides
apply. Then run **on the host**:

```sh
systemctl --user daemon-reload
systemctl --user enable --now lianli-session.service
```

The host unit runs `lianli-session --login-start` inside the box. It waits for the
active graphical login, discovers its environment and starts capture without
requiring the GUI. It starts again for later logins. All application binaries stay
inside the box; this unit does not start or select the hardware daemon.

Its start and stop commands carry the host service invocation ID. Stop verifies
the registered helper's UID and process start time, requests SIGTERM through a
PID handle and waits up to 25 seconds for exit. It refuses a different invocation
or reused PID and does not force-kill the helper. A single private runtime record
retains the last identity so an already-exited invocation can also be checked.

Keep Distrobox's normal host process sharing and user runtime integration.
The helper requires the host system bus at `/run/host/run/dbus/system_bus_socket`,
the same private `/run/user/<uid>` directory in both views, readable desktop-user
process environments, and the compositor sockets and GPU nodes inside the box.
It rejects mismatched runtime directories. A box created with `--unshare-process`
may not expose the desktop environment needed for discovery; preserve host process
sharing for this startup method. See [Distrobox's namespace options](https://distrobox.it/usage/distrobox-assemble/).

Inspect capture startup **on the host** with
`journalctl --user -u lianli-session.service -b -n 100 --no-pager`.
Installation Health checks the host capture unit's effective start and stop commands,
box name, shutdown policy and persistent enablement, and reports its runtime state.
A passed startup check does not establish successful capture or panel playback.
If host inspection is unavailable, the check remains Unverified with the reason.
An uninstalled or inactive disabled helper is reported as optional, not a failed
installation. Existing units generated by version 1.0.0 remain supported.

## Host bridge and service selection

System mode inside a personal Distrobox uses a host system unit running as the
unprivileged account that owns the box. The application binaries remain inside
the box. The unit requires that account's user manager for the rootless container
runtime and uses the host system socket through `/run/host/run/lianli`.

Before selecting System mode, enable lingering manually **on the host**:

```bash
sudo loginctl enable-linger "$(id -u)"
loginctl show-user "$(id -u)" --property=Linger
```

The result must be `Linger=yes`. Then choose **Recheck** in the GUI and switch
to System mode. Lingering lets all of this account's user services run before
login and after logout. The application never enables or disables it for you.
User mode does not require lingering. If you later disable lingering, switch to
User mode first. Otherwise the boxed system daemon can stop after logout.

The unit generator accepts `--system-uid HOST_UID --system-config /absolute/guest/config.json`
alongside `--box`. The configuration must be writable inside the box and separate
from user-mode settings. These options only print a recipe. For managed installation,
stop the daemon and choose **Set up host support** in the box's GUI Settings.
Review the paths and authorize setup, then select User or System mode. This installs
the small host helper and support files without requiring the full native application.
Host support also schedules interrupted-switch recovery at boot.
The bundled helper must be compatible with the host's libraries. A compatibility
failure is reported before installation. A rootless box is entered as its owner,
not as host root or the unrelated native `lianli` account.

The capture helper also discovers the host system socket. Before registration,
it checks the protected host service selection and the socket peer's account.
Inside Distrobox, the selected box owner must have the same numeric UID on the
host and inside the box. An unmapped host account is rejected rather than treated
as the container's `lianli` user. These checks run on connection, not per frame.

Installation Health inspects host files and, when an
installed `host-spawn` binary and the host session bus are available, queries host
service state. It never invokes `distrobox-host-exec` or installs its helper during
a check. Missing bridge prerequisites appear as unverified service findings.
Compositor, GPU and asset visibility still depend on the box's host integration.

`host-spawn` belongs **inside the box**. If it is missing, run this interactively
inside that box and accept Distrobox's installation prompt:

```sh
distrobox-host-exec true
```

Initialize this before starting the daemon. Each daemon launch verifies the host's
root-owned service selection through the bridge, including whether a record is
absent; a container-hidden record cannot bypass the selected hardware account.
Only the selected mode and UID may start once the host has configured a selection.
A direct launch inside the box requests user mode under the container user's UID.
If host system mode is selected, it remains inactive even when that system service
is stopped. Startup output identifies the selected and requested mode/account, or
an unfinished switch. Use the host service controls to select user mode for testing.
A missing bridge or hidden host `/etc/lianli` gives startup guidance instead of
silently applying a private container selection. See [host mode selection](service-modes.md#host-mode-selection).

The host does not need its own copy of `host-spawn`. The binary connects to the
host session bus and asks the host's Flatpak session helper to run commands.
The host therefore needs that helper (provided by Flatpak), a graphical user
session and the systemd tools. If the bridge reports that
`org.freedesktop.Flatpak` is unavailable, repair/install Flatpak on the host using
the host distribution's supported method, then log in again and Recheck.
See [Distrobox's host-execution guide](https://distrobox.it/usage/distrobox-host-exec/).

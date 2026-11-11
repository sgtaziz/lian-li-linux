# NixOS

The repository is a flake with a package and a NixOS module. The module installs the
udev rules, service units, Polkit rule, tmpfiles entries and the `lianli` system
account, which a plain `nix profile` install cannot provide.

## NixOS module

Add the flake to your system flake and import the module:

```nix
{
  inputs.lian-li-linux.url = "git+https://github.com/sgtaziz/lian-li-linux";

  outputs = { nixpkgs, lian-li-linux, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        lian-li-linux.nixosModules.default
        {
          services.lian-li-linux = {
            enable = true;
            evdi.enable = true;
          };
        }
      ];
    };
  };
}
```

| Option | Default | Description |
| --- | --- | --- |
| `enable` | `false` | Install the package, udev rules, units, Polkit rule and `lianli` account |
| `userService.enable` | `false` | Start the user daemon at login for every user, subject to the saved selection |
| `systemService.enable` | `false` | Run the system daemon at boot as `lianli` instead of the user daemon |
| `evdi.enable` | `false` | Load the EVDI kernel module for [desktop-mode LCDs](desktop-backends.md) |
| `package` | flake package | Override the package |

Rebuild with `sudo nixos-rebuild switch`, then log out and back in so `uaccess` applies
to the USB devices. See [Service modes](service-modes.md) for the difference between the
user and system daemon.

## Service mode

NixOS generates `/etc/systemd` read-only and systemd reports its units as `linked`, so
the GUI cannot enable, disable, switch, start or restart the daemon. Pick the mode in the
configuration and rebuild: `userService.enable = true` starts the user daemon at login,
and `systemService.enable = true` starts the system daemon at boot. Set one, not both.

`userService.enable` starts a daemon for every account that logs in, and only one can own
the hardware. On a machine where more than one account logs in, record the owner once,
while no other account is logged in. Otherwise the other accounts' daemons retry every
5 seconds for as long as they are logged in:

```sh
systemctl --user stop lianli-daemon
sudo lianli-control select-mode --scope user --user-uid "$(id -u)"
systemctl --user start lianli-daemon
```

With neither set, start the user daemon yourself each session:

```sh
systemctl --user start lianli-daemon
```

Do not run `systemctl --user enable lianli-daemon`. It links the unit to one store path,
which goes stale on the next update.

### Moving between modes

An existing install keeps its saved mode in `/etc/lianli/service-selection.json`, and a
daemon that is not the selected one exits at once. `select-mode` refuses to run while a
daemon owns the hardware, so stop the running daemon first. A fresh install has no
selection and needs none of this.

Settings do not follow the mode. The user daemon keeps its state in
`${XDG_CONFIG_HOME:-$HOME/.config}/lianli` and the system daemon in `/var/lib/lianli`,
and neither `select-mode` nor a rebuild copies it. Copy it yourself while both daemons
are stopped, or the new mode starts from defaults or from whatever it last saved. The
copy overwrites matching files in the destination, so the steps below back it up first.
Media that a template or LCD setting points at must also be readable by the new account.
See [Configuration and template backups](state-backups.md) for what the directory holds.

From user to system mode:

```sh
systemctl --user stop lianli-daemon
src="${XDG_CONFIG_HOME:-$HOME/.config}/lianli"
sudo install -d -o lianli -g lianli /var/lib/lianli
sudo cp -a /var/lib/lianli "/var/lib/lianli.bak-$(date +%s)"
sudo cp -a "$src/." /var/lib/lianli/
sudo chown -R lianli:lianli /var/lib/lianli
sudo lianli-control select-mode --scope system
```

Rebuild with `systemService.enable = true` and `userService.enable = false`. The
rebuild does not always start the system daemon, so start it or reboot:

```sh
sudo systemctl start lianli-daemon-system
```

From system to user mode:

```sh
sudo systemctl stop lianli-daemon-system
dst="${XDG_CONFIG_HOME:-$HOME/.config}/lianli"
mkdir -p "$dst"
cp -a "$dst" "$dst.bak-$(date +%s)"
sudo cp -a /var/lib/lianli/. "$dst/"
sudo chown -R "$(id -un):$(id -gn)" "$dst"
sudo lianli-control select-mode --scope user --user-uid "$(id -u)"
```

Rebuild with `systemService.enable = false` and `userService.enable = true`, then start
the user daemon or log in again:

```sh
systemctl --user start lianli-daemon
```

The user selection names one UID. Daemons for other users exit while it stands, even
after that user logs out. To hand the hardware to another user, stop the running daemon,
run `sudo lianli-control select-mode --scope user --user-uid UID` with their non-root
UID, then start the daemon in their session.

Rebuild straight after `select-mode`. If the machine reboots in between, the old unit
is still the enabled one and the new selection makes it exit, so no daemon runs. Either
finish the rebuild and start the new daemon as above, or put the old selection back
with `select-mode` and start the old daemon.

Leave out the copy and backup lines to keep each mode's own settings.

Both `git+https://github.com/sgtaziz/lian-li-linux` and `github:sgtaziz/lian-li-linux` work. To build
a fork with commits that aren't upstream, use
`git+https://github.com/<you>/lian-li-linux?submodules=1` so Nix fetches the vendored submodules
from the fork.

## Package only

```sh
nix run git+https://github.com/sgtaziz/lian-li-linux
nix profile add git+https://github.com/sgtaziz/lian-li-linux
```

This gives you `lianli-gui`, `lianli-daemon`, `lianli-control` and `lianli-session`
without udev rules, so devices are only accessible as root. Use the module for a working
install.

## Development

`nix develop` opens a shell with the build inputs, `rustfmt`, `clippy`, `ffmpeg` and
`libsystemd`, which the test suite needs:

```sh
nix develop -c cargo test --workspace
```

## Notes

- Absolute paths to `systemctl`, `journalctl`, `systemd-run`, `pkexec` and the project binaries are
  patched to Nix store and `/run/wrappers` paths at build time.
- The module enables the desktop capture helper for every login, as the packages for
  other distributions do.
- With `userService.enable`, Installation Health reports "Hardware service enabled for
  every user". That is the declared mode. On a shared machine, record the owner as in
  [Service mode](#service-mode).
- `ffmpeg` and `ffprobe` are added to the daemon's `PATH`.
- The system daemon also gets `/run/current-system/sw/bin` on its `PATH`, for `fc-match`,
  `sh`, `nvidia-smi` and `lspci`. A declared user unit gets the login `PATH`, including
  the per-user profiles, so command sensors find the same programs as a shell.
- `libevdi` and the GL/GBM libraries are added to the runpath, along with the driver
  directory in `/run/opengl-driver`.

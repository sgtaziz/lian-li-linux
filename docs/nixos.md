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
| `userService.enable` | `false` | Start the user daemon at login, for every user |
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

Settings do not follow the mode. The user daemon keeps its state in `~/.config/lianli`
and the system daemon in `/var/lib/lianli`, and neither `select-mode` nor a rebuild
copies it. Copy it yourself while both daemons are stopped, or the new mode starts from
defaults or from whatever it last saved. Media that a template or LCD setting points at
must also be readable by the new account. See [Configuration and template
backups](state-backups.md) for what the directory holds.

From user to system mode:

```sh
systemctl --user stop lianli-daemon
sudo install -d -o lianli -g lianli /var/lib/lianli
sudo cp -a ~/.config/lianli/. /var/lib/lianli/
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
mkdir -p ~/.config/lianli
sudo cp -a /var/lib/lianli/. ~/.config/lianli/
sudo chown -R "$(id -un):$(id -gn)" ~/.config/lianli
sudo lianli-control select-mode --scope user --user-uid "$(id -u)"
```

Rebuild with `systemService.enable = false` and `userService.enable = true`, then start
the user daemon or log in again:

```sh
systemctl --user start lianli-daemon
```

Leave out the copy lines to keep each mode's own settings.

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

## Notes

- Absolute paths to `systemctl`, `journalctl`, `systemd-run`, `pkexec` and the project binaries are
  patched to Nix store and `/run/wrappers` paths at build time.
- The module enables the desktop capture helper for every login, as the packages for
  other distributions do.
- With `userService.enable`, Installation Health reports "Hardware service enabled for
  every user". That is the declared mode, and the daemon lock still allows one owner.
- `ffmpeg` and `ffprobe` are added to the daemon's `PATH`.
- The system daemon also gets `/run/current-system/sw/bin` on its `PATH`, for `fc-match`,
  `sh`, `nvidia-smi` and `lspci`. A declared user unit gets the login `PATH`, including
  the per-user profiles, so command sensors find the same programs as a shell.
- `libevdi` and the GL/GBM libraries are added to the runpath, along with the driver
  directory in `/run/opengl-driver`.

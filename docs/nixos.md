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
the GUI cannot enable, disable or switch the daemon. Pick the mode in the configuration
and rebuild: `userService.enable = true` starts the user daemon at login, and
`systemService.enable = true` starts the system daemon at boot. Set one, not both. With
neither set, start the user daemon from the GUI each session.

Do not run `systemctl --user enable lianli-daemon`. It links the unit to one store path,
which goes stale on the next update.

An existing install keeps its saved mode in `/etc/lianli/service-selection.json`, so
update it when moving between modes. A fresh install has no selection and needs neither
step.

From user to system mode, stop the user daemon, record the selection, then rebuild with
`systemService.enable = true` and `userService.enable = false`:

```sh
systemctl --user stop lianli-daemon
sudo lianli-control select-mode --scope system
```

From system to user mode, rebuild with `systemService.enable = false` and
`userService.enable = true`, then record the selection:

```sh
sudo lianli-control select-mode --scope user --user-uid "$(id -u)"
```

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
- A declared daemon also gets `/run/current-system/sw/bin` on its `PATH`, for `fc-match`,
  `sh`, `nvidia-smi` and `lspci`.
- `libevdi` and the GL/GBM libraries are added to the runpath, along with the driver
  directory in `/run/opengl-driver`.

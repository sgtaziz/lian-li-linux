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
| `systemService.enable` | `false` | Run the system daemon at boot as `lianli` instead of the user daemon |
| `evdi.enable` | `false` | Load the EVDI kernel module for [desktop-mode LCDs](desktop-backends.md) |
| `package` | flake package | Override the package |

Rebuild with `sudo nixos-rebuild switch`, then log out and back in so `uaccess` applies
to the USB devices. See [Service modes](service-modes.md) for the difference between the
user and system daemon.

## Service mode

Without `systemService.enable`, choose **User (at login)** in the GUI on first launch.
That enables the user daemon in `~/.config/systemd/user`, which works as on other
distributions.

NixOS generates `/etc/systemd` read-only, so the GUI cannot enable or disable the
system daemon. Set `systemService.enable = true` for system mode and rebuild. The GUI's
mode switch is not available while the system daemon is declared.

An existing install keeps its saved mode in `/etc/lianli/service-selection.json`, so
update it when moving between modes. A fresh install has no selection and needs neither
step.

From user to system mode, stop the user daemon, record the selection, then rebuild with
`systemService.enable = true`:

```sh
systemctl --user disable --now lianli-daemon
sudo lianli-control select-mode --scope system
```

From system to user mode, rebuild with `systemService.enable = false`, then record the
selection and choose **User (at login)** in the GUI:

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
- The desktop capture helper starts from its XDG autostart entry, as on other
  distributions.
- `ffmpeg` and `ffprobe` are added to the daemon's `PATH`.
- `libevdi` and the GL/GBM libraries are added to the runpath, along with the driver
  directory in `/run/opengl-driver`.

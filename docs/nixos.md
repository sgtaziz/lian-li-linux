# NixOS

The repository is a flake with a package and a NixOS module. The module installs the
udev rules, service units, Polkit rule, tmpfiles entries and the `lianli` system
account, which a plain `nix profile` install cannot provide.

## NixOS module

Add the flake to your system flake and import the module:

```nix
{
  inputs.lian-li-linux.url = "github:sgtaziz/lian-li-linux";

  outputs = { nixpkgs, lian-li-linux, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      modules = [
        lian-li-linux.nixosModules.default
        {
          services.lian-li-linux = {
            enable = true;
            serviceMode = "user";
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
| `serviceMode` | `null` | `"user"` or `"system"` starts that daemon automatically. `null` leaves both stopped so you can choose in the GUI |
| `evdi.enable` | `false` | Load the EVDI kernel module for [desktop-mode LCDs](desktop-backends.md) |
| `package` | flake package | Override the package |

Rebuild with `sudo nixos-rebuild switch`, then log out and back in so `uaccess` applies
to the USB devices. See [Service modes](service-modes.md) for the difference between the
user and system daemon.

## Package only

```sh
nix run github:sgtaziz/lian-li-linux
nix profile add github:sgtaziz/lian-li-linux
```

This gives you `lianli-gui`, `lianli-daemon`, `lianli-control` and `lianli-session`
without udev rules, so devices are only accessible as root. Use the module for a working
install.

## Notes

- Absolute paths to `systemctl`, `systemd-run`, `pkexec` and the project binaries are
  patched to Nix store and `/run/wrappers` paths at build time.
- `ffmpeg` and `ffprobe` are added to the binaries' `PATH`.
- `libevdi` and the GL/GBM libraries are added to the runpath, along with the driver
  directory in `/run/opengl-driver`.

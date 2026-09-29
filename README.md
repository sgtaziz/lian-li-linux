<p align="center">
  <img src="assets/icons/icon.svg" width="128" height="128" alt="Lian Li Linux">
</p>

<h1 align="center">Lian Li Linux</h1>

<p align="center">
  Open-source Linux replacement for L-Connect 3.<br>
  Fan speed control, RGB/LED effects, LCD streaming, and sensor gauges for all Lian Li devices.
</p>

### AI Disclaimer

Generative AI was used during development in two areas:

- **Reverse engineering.** AI assisted with analyzing USB packet captures and vendor
  software to understand device communication protocols — decoding packet structures,
  identifying encryption and compression schemes, cross-referencing control opcodes, and translating findings to Rust.
- **Frontend UI.** AI helped scaffold the Vue/TypeScript frontend for the Tauri GUI, including
  component layout, styling, and state wiring.

All AI output is reviewed personally before being committed. Protocols for devices I
own were validated against real hardware, others rely on community testing and feedback. Bug reports are welcome.

---

## Supported Devices

### HID

| Device | Fan Control | RGB | LCD | Pump | Tested |
|--------|:-----------:|:---:|:---:|:----:|:------:|
| UNI FAN SL / AL / SL Infinity / SL V2 / AL V2 | 4 groups | Yes | - | - | Yes |
| UNI FAN TL Controller | 4 ports | Yes | - | - | Yes |
| UNI FAN TL LCD | 4 ports | Yes | 400x400 | - | Yes |
| Galahad II Trinity AIO | Yes | Yes | - | Yes | Yes |
| HydroShift LCD AIO | Yes | Yes | 480x480 | Yes | Yes |
| Galahad II LCD / Vision AIO | Yes | Yes | 480x480 | Yes | Yes |
| Strimer Plus (wired) | - | Yes | - | - | Yes |

Edge Hub Advanced (`39b0:0101`) exposes read-only identity, firmware version and three temperatures on its device card. Unavailable or stale sensors are marked explicitly. Physical sensor accuracy still needs hardware validation; no fan, RGB or firmware-update commands are sent.

### Wireless (via TX/RX dongle)

| Device | Fan Control | RGB | LCD | Pump | Tested |
|--------|:-----------:|:---:|:---:|:----:|:------:|
| UNI FAN TL V2 (LCD / LED) | Yes | Yes | 400x400 | - | Yes |
| UNI FAN SL V3 (LCD / LED) | Yes | Yes | 400x400 | - | Yes |
| UNI FAN SL-INF | Yes | Yes | - | - | Yes |
| UNI FAN CL / RL120 | Yes | Yes | - | - | - |
| HydroShift II LCD-C (Wireless) | Yes | Yes | - | Yes | Yes |
| HydroShift II LCD-S (Wireless) | Yes | Yes | - | Yes | Yes |
| Strimer Plus Wireless | - | Yes | - | - | Yes |
| Lancool 217 Wireless | - | Yes | - | - | - |
| Lancool V150 Wireless | Yes | Yes | - | - | - |
| Universal Screen 8.8" Wireless | - | Yes | - | - | - |

Both V1 (VID 0x0416) and V2 (VID 0x1A86) wireless dongles are supported. Binding devices is supported through the GUI, which reports command completion or failure. Wireless control requires both TX and RX dongles. Saved fan and lighting settings do not automatically bind an unbound group on startup, use the GUI to bind it explicitly.

Wireless SL V3 fans support hardware motherboard PWM sync. Other wireless fans can follow a selected Linux PWM header; if that source is missing or becomes unreadable, they run at full speed until it returns.

> **Note:** Wireless devices with LCDs still need to be plugged in via USB to control the LCD. LCD cannot be controlled through wireless dongle alone.

### Software RGB effects

The RGB page offers software effects for supported wireless devices and verified wired frame-capable controllers. Choose an effect per zone, adjust colors, speed, brightness and direction, then save. Supported effects include Rainbow, Rainbow Morph, Breathing, Color Cycle, Runway, Meteor, Wave, Tide, Ping Pong and Twinkle, plus Static and Off. Direct mode edits individual LEDs; presets preserve either effects or direct colors.

Wireless devices and compatible wired receivers loop uploaded animations. Other supported wired LED controllers receive frames from the daemon, capped at 20 fps, and require the daemon to remain running. Large uploads may use fewer frames to fit device memory without changing the animation duration. Hardware effect controllers such as TL retain their existing modes.

When a device is connected both ways at once it shows once as a wireless group and the duplicate
wired entry is hidden, control and telemetry go over RF. Merging wired LCD fans into their wireless
group needs the V2 dongle, V1 dongles show both entries.

### USB

| Device | Fan Control | RGB | LCD | Pump | Tested |
|--------|:-----------:|:---:|:---:|:----:|:------:|
| HydroShift II LCD Circle | Yes | Yes | 480x480 | Yes | Yes |
| HydroShift II LCD Square | Yes | Yes | 480x480 | Yes | Yes |
| Lancool 207 Digital | - | - | 720x1472 | - | Yes |
| Universal Screen 8.8" | - | Yes | 480x1920 | - | Yes |
| Vision 9.2" | - | - | 464x1920 | - | Yes |
| TL Flex LCD | Yes | Yes | 400x400 | - | Yes |
| SL Infinity Flex LCD | Yes | Yes | 400x400 | - | - |
| HydroShift II OLED Curve | - | Yes | 1080x2288 | Yes | Yes |

### Desktop Mode (Virtual Display)

HydroShift II, Lancool 207 Digital and Universal Screen 8.8" can extend your desktop.
Hyprland uses native headless monitors. Other sessions try Hermes-KMS first, then
EVDI. Desktop capture needs a logged-in graphical session, even with a system
daemon. These optional backends are not required for cooling, RGB or LCD playback.
See [desktop setup and requirements](docs/desktop-backends.md).

If you have tested a device not marked above, please
[report your results](https://github.com/sgtaziz/lian-li-linux/issues) with a diagnostic export.

## Installing

### Arch Linux (AUR)

```sh
yay -S lianli-linux-git
```

To build the package directly:

```sh
git clone --recurse-submodules https://github.com/sgtaziz/lian-li-linux.git
cd lian-li-linux/packaging/archlinux
makepkg -si
```

### Fedora (COPR)

For immutable Fedora distributions such as Bazzite, use the
[recommended Distrobox installation](#distrobox-and-immutable-hosts) below.

Enable RPM Fusion Free for full FFmpeg with libx264, then install the project package:

```sh
sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo dnf copr enable sgtaziz/lian-li-linux
sudo dnf install lian-li-linux
```

### Debian and Ubuntu

Use the release artifact matching your distribution: Ubuntu 24.04, Ubuntu 26.04
or Debian 13, amd64. These packages are distribution-specific because their linked
libraries differ. See [download verification and installation](docs/debian-packages.md).

### Distrobox and immutable hosts

**We strongly recommend Distrobox on immutable distributions such as Bazzite,
Fedora Silverblue and Kinoite.** It keeps the application and its dependencies
inside a Fedora container, avoiding application package layering and dependency
conflicts with the host image.

Follow the [Distrobox installation guide](docs/distrobox.md), including host USB
rules, the host bridge and user/system service setup. Complete the host setup
steps too: installing the package inside the box alone is not enough. Optional
desktop kernel modules still belong on the host.

[Native package layering on immutable Fedora](docs/immutable-host.md) is an
advanced alternative for users who specifically need a host installation. It
changes the host deployment and requires reboots. Prefer Distrobox for normal use.

### From Source

See [build dependencies and installation](docs/building-from-source.md).
`cargo build --release` builds the daemon, GUI, control helper and session helper,
including the frontend.

## First launch

Open **Lian Li Linux** after installation. **Installation Health** checks setup
and links to repair instructions. Fresh native installs leave both daemon services
stopped: choose **User (at login)** or **System (at boot)** and select **Enable and
start**. Only one mode can own the hardware.

Use **Settings → Configuration** to manage services, transfer settings/media,
restore backups and manage storage. The system daemon needs access to every LCD
asset, including images, video and fonts referenced by templates.

- [Service setup, switching and recovery](docs/service-modes.md)
- [USB permissions and udev rules](docs/usb-permissions.md)
- [LCD assets and managed storage](docs/lcd-assets.md)
- [Startup image uploads](docs/startup-images.md)
- [Configuration and template backups](docs/state-backups.md)
- [OpenRGB and separate AL V2 regions](docs/openrgb.md)

## Configuration

The GUI saves changes through the daemon. User configuration lives in
`~/.config/lianli/`; system configuration lives in `/var/lib/lianli/`.

**Hardware video acceleration** is disabled by default. Enable it in Settings and
save to recreate active video streams without restarting the daemon. See
[hardware video](docs/hardware-video.md) for supported paths and diagnostics.

LCD cards also offer [pixel conditioning](docs/pixel-conditioning.md), with
cancellation and restoration of the previous display.

## Help and diagnostics

Start with [troubleshooting](docs/troubleshooting.md). Before opening an
[issue](https://github.com/sgtaziz/lian-li-linux/issues), export a diagnostic report
from **Installation Health**. The export requires daemon logs and includes logs
from the last daemon startup. Review the report before sharing it.

## Architecture

The Rust daemon owns hardware, cooling and media playback. The Tauri/Vue GUI
communicates with it over a Unix socket. `lianli-control` manages service ownership
and state transfer; `lianli-session` captures desktop outputs in the graphical
session.

## License

MIT. See [LICENSE](LICENSE).

This project is not affiliated with Lian Li Industrial Co., Ltd.
Protocol information was obtained through reverse engineering for interoperability purposes.

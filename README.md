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

Devices in desktop/display mode (HydroShift II, Lancool 207 Digital, Universal Screen 8.8") are
additionally driven as a native secondary monitor via [evdi](https://github.com/DisplayLink/evdi).
The daemon auto-attaches an evdi virtual output on detection, the device shows up in your
compositor's display settings with its real EDID, and any window can be dragged onto it.

Requirements:
- `evdi-dkms` — bundles the userspace library (required to link the daemon) and the kernel
  module (required at runtime for virtual display attach). On Arch this is the `evdi-dkms` AUR
  package; on Debian/Ubuntu both pieces are packaged separately (`libevdi0-dev` + `evdi-dkms`).
- System `ffmpeg` libraries (libavcodec/libavformat/libswscale) for H.264 encoding — already
  pulled in by the base `ffmpeg` dependency.

The daemon will still start without the kernel module loaded, but desktop-mode devices (HydroShift II,
Lancool 207, Universal Screen 8.8") won't get attached as virtual displays until the module is present.

`/sys/devices/evdi/add` is root-only by default; the package ships a udev rule that grants write access to it (and a `modules-load.d` drop-in that auto-loads the `evdi` module at boot), so the daemon creates and opens its own evdi nodes with no root setup step.

If you've tested a device that isn't marked as tested above, please [open an issue or PR](https://github.com/sgtaziz/lian-li-linux/issues) to update this table.

## Architecture

```
lianli-daemon          Daemon (user or system service) - fan control loop + LCD streaming
  lianli-devices       HID/USB device drivers
  lianli-transport     USB bulk transport (wireless protocol, display streaming)
  lianli-media         Image/video/GIF encoding, sensor gauge rendering
  lianli-shared        IPC types, config schema, device IDs

lianli-gui             Tauri desktop app (Rust + Vue) - connects to daemon via Unix socket
```

The daemon runs as either a per-user or system systemd service (see [Service modes](#service-modes)), neither is enabled automatically. USB access is granted via udev rules (no root required). The GUI connects over `$XDG_RUNTIME_DIR/lianli-daemon.sock` (per-user daemon) or `/run/lianli/lianli-daemon.sock` (system daemon) and auto-detects which.

## Installing

### Arch Linux (AUR)

```bash
yay -S lianli-linux-git
```

You can also build from the PKGBUILD in case AUR is inaccessible:
```bash
git clone --recurse-submodules https://github.com/sgtaziz/lian-li-linux.git && cd lian-li-linux/packaging/archlinux
makepkg -si
```

The package installs binaries, udev rules, both systemd units, and creates the `lianli` system user/group automatically. Reload udev, then pick a service mode:
```bash
sudo udevadm control --reload-rules && sudo udevadm trigger
```

See [Service modes](#service-modes) for the per-user vs system choice and how to enable each.

### Fedora (COPR)

```bash
# libx264 (H.264 LCD streaming) is GPL, ffmpeg with it is only available in rpmfusion free repo. Enable it first
sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
# Then the project repo
sudo dnf copr enable sgtaziz/lian-li-linux
sudo dnf install lian-li-linux
```

This installs binaries, udev rules, both systemd units, the `lianli` system user/group, desktop entry, and icons, and pulls in full `ffmpeg` (with libx264) from rpmfusion. It does **not** auto-start the daemon. Pick a mode in [Service modes](#service-modes).

Desktop-mode devices (HydroShift II, Lancool 207, Universal Screen 8.8") additionally need the `evdi` kernel module:
```bash
sudo dnf copr enable crashdummy/Displaylink
sudo dnf install displaylink
```

### Distrobox / toolbx (containers)

On Bazzite and other immutable systems, the recommended way to add software is a distrobox or toolbx container rather than layering onto the base. lian-li-linux runs fine in one — just make sure your USB devices are exposed to the box so the daemon can see them.

Inside the container:
```bash
# 1. Enable rpmfusion (full ffmpeg with libx264)
sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
# 2. Enable the project COPR
sudo dnf copr enable sgtaziz/lian-li-linux
# 3. Install, skipping Recommends — displaylink pulls a DKMS kernel-module build
#    that triggers dracut, which fails in a container. Kernel modules don't belong
#    in a box anyway.
sudo dnf install --setopt=install_weak_deps=False lian-li-linux
```

Don't enable `crashdummy/Displaylink` inside the box either, same reason.

A container doesn't run systemd, so the shipped `lianli-daemon.service` can't manage the daemon from inside the box. To start it on login, create a user systemd unit on the **host** that enters the box, e.g. `~/.config/systemd/user/lianli-daemon.service`:
```ini
[Unit]
Description=Lian Li Daemon (distrobox)
After=graphical-session.target

[Service]
ExecStart=/usr/bin/distrobox-enter -n BOX -- lianli-daemon
Restart=on-failure

[Install]
WantedBy=default.target
```
Replace `BOX` with your container name (check the path with `command -v distrobox-enter`), then:
```bash
systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon.service
```
Run the GUI with `distrobox-enter -n BOX -- lianli-gui`.

### Immutable Fedora (Bazzite)

Prefer the distrobox method above. If you need to install directly on the base instead, `dnf install` doesn't apply on rpm-ostree. Packages layer into a new deployment that only takes effect after a reboot, and the daemon won't start on its own.

```bash
# 1. Enable rpmfusion (full ffmpeg with libx264)
sudo rpm-ostree install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo systemctl reboot

# 2. After reboot: add the project repo and install
sudo curl --output-dir /etc/yum.repos.d/ --remote-name \
  https://copr.fedorainfracloud.org/coprs/sgtaziz/lian-li-linux/repo/fedora-$(rpm -E %fedora)/sgtaziz-lian-li-linux-fedora-$(rpm -E %fedora).repo
sudo rpm-ostree install lian-li-linux
sudo systemctl reboot

# 3. After reboot: start the daemon
systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon.service
```


### From Source

1) Clone the repo and submodules:
```bash
git clone --recurse-submodules https://github.com/sgtaziz/lian-li-linux.git && cd lian-li-linux
```
> If you already cloned without `--recurse-submodules`, run: `git submodule update --init --recursive`

2) Install dependencies:
- **Rust** (stable)
- **npm**
- **ffmpeg** and **ffprobe** in `PATH` (for video/GIF decoding)
- **System libraries:**

```bash
# Arch
sudo pacman -S libusb ffmpeg fontconfig mesa libxkbcommon wayland libx11 libinput libdrm \
  libjpeg-turbo clang cmake pkg-config nasm npm \
  webkit2gtk-4.1 gtk3 glib2 libsoup3 libayatana-appindicator librsvg
yay -S evdi-dkms             # AUR — evdi-dkms bundles libevdi + DKMS module

# Ubuntu / Debian
sudo apt install libusb-1.0-0-dev libudev-dev libfontconfig-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libinput-dev libdrm-dev \
  libgl-dev libegl-dev clang cmake pkg-config ffmpeg nasm npm \
  libavcodec-dev libavformat-dev libswscale-dev libavutil-dev \
  libevdi0-dev \
  libwebkit2gtk-4.1-dev libglib2.0-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev
sudo apt install evdi-dkms  # optional, only needed at runtime for desktop-mode devices

# Fedora
sudo dnf install libusb1-devel fontconfig-devel \
  libxkbcommon-devel wayland-devel libX11-devel libinput-devel libdrm-devel \
  mesa-libGL-devel mesa-libEGL-devel clang cmake pkg-config ffmpeg \
  ffmpeg-devel nasm npm \
  webkit2gtk4.1-devel gtk3-devel glib2-devel libsoup3-devel \
  libappindicator-gtk3-devel librsvg2-devel
# evdi is not packaged in Fedora repos — build libevdi from source to link the daemon:
#   https://github.com/DisplayLink/evdi  (evdi-dkms is only needed at runtime)
# You can also download https://github.com/displaylink-rpm/displaylink-rpm instead
# Make sure to replace ffmpeg-free with ffmpeg if ffmpeg-free is installed
```

3) Build:
```bash
cargo build --release
```

The GUI crate's build script runs `npm install` + `npm run build` automatically when the frontend
sources are newer than the built `dist/`.

Binaries: `target/release/lianli-daemon` and `target/release/lianli-gui`

4) Install udev rules (required for USB access without root):
```bash
sudo install -Dm644 packaging/udev/60-lianli.rules /usr/lib/udev/rules.d/60-lianli.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
# If evdi is already loaded, apply the new evdi chmod rule without a reboot:
[ -e /sys/module/evdi ] && sudo udevadm trigger --action=add /sys/module/evdi
```

> Install to `/usr/lib/udev/rules.d/` (the vendor location, same as the package) — **not** `/etc/udev/rules.d/`. A file in `/etc` with the same name would shadow the packaged one and silently override it.

For headless operation, run the [system service](#service-modes). This allows control even when no users are logged in.

5) Install binaries, service units, and the system user/group:
```bash
sudo install -Dm755 target/release/lianli-daemon /usr/bin/lianli-daemon
sudo install -Dm755 target/release/lianli-gui /usr/bin/lianli-gui

sudo install -Dm644 packaging/systemd/lianli-daemon.service /usr/lib/systemd/user/lianli-daemon.service
sudo install -Dm644 packaging/systemd/lianli-daemon-system.service /usr/lib/systemd/system/lianli-daemon-system.service
sudo install -Dm644 packaging/sysusers.d/lianli.conf /usr/lib/sysusers.d/lianli.conf
sudo install -Dm644 packaging/tmpfiles.d/lianli.conf /usr/lib/tmpfiles.d/lianli.conf
sudo systemd-sysusers lianli.conf
sudo systemd-tmpfiles --create lianli.conf

# Auto-load evdi at boot (for desktop-mode LCD support)
sudo install -Dm644 packaging/modules-load.d/lianli-evdi.conf /usr/lib/modules-load.d/lianli-evdi.conf
systemctl --user daemon-reload
sudo systemctl daemon-reload
```

Now enable one service — see [Service modes](#service-modes). A default config is created on first run at `~/.config/lianli/config.json` (user service) or `/var/lib/lianli/config.json` (system service).

6) Install desktop entry and icons:
```bash
# Install icons
for size in 32x32 128x128 256x256 scalable; do mkdir -p ~/.local/share/icons/hicolor/$size/apps; done
cp assets/icons/32x32.png ~/.local/share/icons/hicolor/32x32/apps/com.sgtaziz.lianlilinux.png
cp assets/icons/128x128.png ~/.local/share/icons/hicolor/128x128/apps/com.sgtaziz.lianlilinux.png
cp assets/icons/128x128@2x.png ~/.local/share/icons/hicolor/256x256/apps/com.sgtaziz.lianlilinux.png
cp assets/icons/icon.svg ~/.local/share/icons/hicolor/scalable/apps/com.sgtaziz.lianlilinux.svg

# Install desktop entry
cp packaging/desktop/com.sgtaziz.lianlilinux.desktop ~/.local/share/applications/
update-desktop-database ~/.local/share/applications/
```

## Udev rules

The default rules grant device access with **no manual setup**:
- The active logged-in user gets read/write via `uaccess` — used by the per-user daemon.
- The `lianli` system user (auto-created at install) gets read/write via the `lianli` group — used by the optional [system service](#service-modes) for headless control.

## Service modes

The daemon ships as two systemd units. **Neither is enabled automatically**. Pick one at install (not both, or they'll fight over the same USB devices). The GUI auto-detects whichever is running.

> **Upgrading from an older package?** Previous versions force-enabled the user service in the **global** scope. That lingers across the upgrade, and `systemctl --user disable` alone won't undo it (you'll get a "still started automatically" warning). Clear it first, then pick a mode:
> ```bash
> sudo systemctl --global disable lianli-daemon.service
> systemctl --user disable lianli-daemon.service
> ```

**Per-user.** Runs as your user, reads `~/.config/lianli/config.json`. Best for multi-user systems (each user has their own profile/LCDs).
```bash
systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon.service
```

**System (headless / single-user).** Runs as the `lianli` system user at boot — no login required. Config lives at `/var/lib/lianli/config.json`.
```bash
sudo systemctl enable --now lianli-daemon-system.service
```
From-source installs must create the user first (packaged installs do it automatically via `sysusers.d`):
```bash
sudo groupadd -r lianli && sudo useradd -r -g lianli -d / -s /usr/sbin/nologin -c "Lian Li daemon" lianli
```

> **LCD media access:** the system daemon reads media files (videos/GIFs/PNGs referenced by your LCD config) directly as the `lianli` user. It can only open files `lianli` has filesystem permission to reach, so if your home dir is hardened to `0700` (`drwx------`), put your media somewhere `lianli` can read (e.g. `/var/lib/lianli/media/`, or any world-readable path). The per-user daemon has no such limit since it runs as you.

### Migrating between modes

Config is stored per mode (user: `~/.config/lianli/`, system: `/var/lib/lianli/`). To keep your settings — profiles, fan curves, RGB presets — when switching, copy the whole directory across and fix ownership:

```bash
# user to system
sudo cp -a ~/.config/lianli/. /var/lib/lianli/
sudo chown -R lianli:lianli /var/lib/lianli

# system to user
cp -a /var/lib/lianli/. ~/.config/lianli/
sudo chown -R $USER:$USER ~/.config/lianli
```

Then enable the other unit and disable the current one (the shared lock refuses to let both run, so stop the old one first). If the old one was the user service and won't disable, see the upgrade note above.

## Configuration

The daemon reads its config from `~/.config/lianli/config.json` (per-user service) or `/var/lib/lianli/config.json` (system service) — see [Service modes](#service-modes). The GUI edits this file via the daemon's IPC socket. LCD targets, fan curves, and speed modes are all configured through the GUI.

## Pixel conditioning

Each LCD card has a pixel-conditioning control with 15, 30, 60, and 120-minute presets. The daemon generates a five-second pattern at the panel's native size: alternating black/white, changing grayscale noise, and solid color phases. There is no bundled video or download. Preparation finishes before the display changes; use Cancel to abandon preparation or Stop to restore the previous display.

```bash
lianli-daemon lcd clean --minutes 30
lianli-daemon lcd clean --device-id 'hid:1-2:1.0#0' --minutes 15
```

Omitting `--device-id` selects active configured LCD targets. The `#0` suffix identifies the configuration entry, not the physical USB port. Ctrl+C or SIGTERM cancels preparation or stops the session started by that CLI process. CLI durations must be positive; values above 255 minutes are capped.

Conditioning uses 75% brightness and restores the configured brightness afterward (100% when omitted). Config reload cancels conditioning before applying new entries. On daemon shutdown, supported LCD backlights are turned off; startup reapplies configured brightness. This routine exercises pixels; it does not guarantee recovery from retention or panel damage.

## Troubleshooting

**Daemon won't start / no devices found:**
```bash
# Check udev rules are loaded
sudo udevadm test /sys/bus/usb/devices/<your-device>

# Check daemon logs (whichever mode you use)
journalctl --user -u lianli-daemon -f          # per-user service
sudo journalctl -u lianli-daemon-system -f     # system service
```

Device access uses `uaccess` (per-user daemon) or the `lianli` group (system service). If you get permission errors or a device isn't detected, confirm the rules are installed and re-triggered. See [Udev rules](#udev-rules) and [Service modes](#service-modes).

**GUI says "Daemon offline":**
```bash
# Verify the daemon is running (whichever mode you use)
systemctl --user status lianli-daemon          # per-user
sudo systemctl status lianli-daemon-system     # system

# Check the socket exists (GUI auto-detects either)
ls -la "$XDG_RUNTIME_DIR/lianli-daemon.sock" /run/lianli/lianli-daemon.sock 2>/dev/null
```

**GUI won't launch (blank window / webview errors):**

The Tauri GUI depends on WebKit2GTK. Ensure `webkit2gtk-4.1` is installed and your GPU drivers support it. On Wayland with NVIDIA, `WEBKIT_DISABLE_COMPOSITING_MODE=1` may help as a workaround.

**Permission denied on USB device:**
```bash
# Re-trigger udev after plugging in device
sudo udevadm trigger
```

## License

MIT. See [LICENSE](LICENSE).

This project is not affiliated with Lian Li Industrial Co., Ltd.
Protocol information was obtained through reverse engineering for interoperability purposes.

<p align="center">
  <img src="assets/icons/icon.svg" width="128" height="128" alt="Lian Li Linux">
</p>

<h1 align="center">Lian Li Linux</h1>

<p align="center">
  Open-source Linux replacement for L-Connect 3.<br>
  Fan speed control, RGB/LED effects, LCD streaming, and sensor gauges for all Lian Li devices.
</p>

---

## Supported Devices

### HID

| Device | Fan Control | RGB | LCD | Pump | Tested |
|--------|:-----------:|:---:|:---:|:----:|:------:|
| UNI FAN SL / AL / SL Infinity / SL V2 / AL V2 | 4 groups | Yes | - | - | Yes |
| UNI FAN TL Controller | 4 ports | Yes | - | - | Yes |
| UNI FAN TL LCD | 4 ports | Yes | 400x400 | - | Yes |
| Galahad II Trinity AIO | Yes | Yes | - | Yes | - |
| HydroShift LCD AIO | Yes | Yes | 480x480 | Yes | Yes |
| Galahad II LCD / Vision AIO | Yes | Yes | 480x480 | Yes | Yes |

\* Galahad II LCD / Vision uses the same driver as HydroShift LCD AIO.

### Wireless (via TX/RX dongle)

| Device | Fan Control | RGB | LCD | Pump | Tested |
|--------|:-----------:|:---:|:---:|:----:|:------:|
| UNI FAN TL V2 (LCD / LED) | Yes | Yes | 400x400 | - | Yes |
| UNI FAN SL V3 (LCD / LED) | Yes | Yes | 400x400 | - | Yes |
| UNI FAN SL-INF | Yes | Yes | - | - | Yes |
| UNI FAN CL / RL120 | Yes | Yes | - | - | - |
| HydroShift II LCD-C (Wireless) | Yes | Yes | - | Yes | - |
| HydroShift II LCD-S (Wireless) | Yes | Yes | - | Yes | - |
| Strimer Plus Wireless | - | Yes | - | - | Yes |
| Lancool 217 Wireless | - | Yes | - | - | - |
| Lancool V150 Wireless | Yes | Yes | - | - | - |
| Universal Screen 8.8" Wireless | - | Yes | - | - | - |

Both V1 (VID 0x0416) and V2 (VID 0x1A86) wireless dongles are supported. Binding devices is supported through the GUI.

> **Note:** Wireless devices with LCDs still need to be plugged in via USB to control the LCD. LCD cannot be controlled through wireless dongle alone.

### USB (Standalone LCD)

| Device | LCD | Tested | Notes |
|--------|:---:|:------:|-------|
| HydroShift II LCD Circle | 480x480 | Yes | |
| HydroShift II LCD Square | 480x480 | Yes | |
| Lancool 207 Digital | 1472x720 | Yes | |
| Universal Screen 8.8" | 1920x480 | Yes | |
| Universal Screen 8.8" LED Ring | - | Yes | RGB control supported |

Devices stuck in desktop/display mode are detected and can be switched back to LCD mode via the GUI.

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

`/sys/devices/evdi/add` is root-only by default; the package ships a udev rule that grants the
active user write access to it (and a `modules-load.d` drop-in that auto-loads the `evdi` module
at boot), so the per-user daemon creates and opens its own evdi nodes with no root setup step.

### Other

| Device | RGB | Tested |
|--------|:---:|:------:|
| Strimer Plus (wired) | Yes | - |

If you've tested a device that isn't marked as tested above, please [open an issue or PR](https://github.com/sgtaziz/lian-li-linux/issues) to update this table.

## Architecture

```
lianli-daemon          User service - fan control loop + LCD streaming
  lianli-devices       HID/USB device drivers
  lianli-transport     USB bulk transport (wireless protocol, display streaming)
  lianli-media         Image/video/GIF encoding, sensor gauge rendering
  lianli-shared        IPC types, config schema, device IDs

lianli-gui             Slint desktop app - connects to daemon via Unix socket
```

The daemon runs as a user systemd service. USB access is granted via udev rules (no root required).
The GUI connects over `$XDG_RUNTIME_DIR/lianli-daemon.sock`.

## Installing

### Arch Linux (AUR)

```bash
yay -S lianli-linux-git
```

Or with any AUR helper (`paru`, `trizen`, etc.). This installs binaries, udev rules, the systemd user service, desktop entry, and icons. The package also globally enables `lianli-daemon.service` and attempts to start it in the current session — no manual `systemctl` step is required.

The daemon runs as a systemd user service and reads `~/.config/lianli/config.json`. If you want it to stay active when no desktop session is logged in, enable linger: `sudo loginctl enable-linger $USER`.

### From Source

1) Clone the repo and submodules:
```bash
git clone --recurse-submodules https://github.com/sgtaziz/lian-li-linux.git && cd lian-li-linux
```
> If you already cloned without `--recurse-submodules`, run: `git submodule update --init --recursive`

2) Install dependencies:
- **Rust** (stable, 1.75+)
- **ffmpeg** and **ffprobe** in `PATH` (for video/GIF decoding)
- **System libraries:**

```bash
# Arch
sudo pacman -S hidapi libusb ffmpeg fontconfig mesa libxkbcommon wayland libx11 libinput libdrm clang cmake pkg-config nasm
yay -S evdi-dkms         # AUR - bundles both libevdi (required) and the DKMS kernel module

# Ubuntu / Debian
sudo apt install libhidapi-dev libusb-1.0-0-dev libudev-dev libfontconfig-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libinput-dev libdrm-dev \
  libgl-dev libegl-dev clang cmake pkg-config ffmpeg nasm \
  libavcodec-dev libavformat-dev libswscale-dev libavutil-dev \
  libevdi0-dev              # required to build/link the daemon
sudo apt install evdi-dkms  # optional, only needed at runtime for desktop-mode devices

# Fedora
sudo dnf install hidapi-devel libusb1-devel fontconfig-devel \
  libxkbcommon-devel wayland-devel libX11-devel libinput-devel libdrm-devel \
  mesa-libGL-devel mesa-libEGL-devel clang cmake pkg-config ffmpeg \
  ffmpeg-devel nasm
# evdi is not packaged in Fedora repos — build libevdi from source to link the daemon:
#   https://github.com/DisplayLink/evdi  (evdi-dkms is only needed at runtime)
```

3) Build:
```bash
cargo build --release
```

Binaries: `target/release/lianli-daemon` and `target/release/lianli-gui`

4) Install udev rules (required for USB access without root):
```bash
sudo cp packaging/udev/99-lianli.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
# If evdi is already loaded, apply the new evdi chmod rule without a reboot:
[ -e /sys/module/evdi ] && sudo udevadm trigger --action=add /sys/module/evdi
```

5) Install and start the daemon:
```bash
# Copy binary
sudo install -Dm755 target/release/lianli-daemon /usr/bin/lianli-daemon

# Install user service
sudo install -Dm644 packaging/systemd/lianli-daemon.service /usr/lib/systemd/user/lianli-daemon.service

# Auto-load evdi at boot (for desktop-mode LCD support)
sudo install -Dm644 packaging/modules-load.d/lianli-evdi.conf /usr/lib/modules-load.d/lianli-evdi.conf

systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon.service
```

A default config is created automatically at `~/.config/lianli/config.json` on first run.

6) Install the GUI (optional):
```bash
cp target/release/lianli-gui ~/.local/bin/

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

### With Docker

1) Build the Docker image:
```bash
docker build -f docker/build.Dockerfile -t lianli-linux-builder \
  --build-arg USER_ID="$(id -u)" \
  --build-arg GROUP_ID="$(id -g)" \
  .
```
2) Build the project:
```bash
docker run --rm -it \
  -v "$PWD:/work" \
  -v "$PWD/target:/work/target" \
  -v "$PWD/.cache/cargo-registry:/home/builder/.cargo/registry" \
  -v "$PWD/.cache/cargo-git:/home/builder/.cargo/git" \
  lianli-linux-builder
```

Then follow steps 4-6 from "From Source" above.

## Configuration

The daemon reads `~/.config/lianli/config.json`. The GUI edits this file via the daemon's IPC socket.

### LCD Streaming

Each LCD entry specifies a target device (by serial), media type, and orientation:

| Type | Description |
|------|-------------|
| `image` | Static image (JPEG, PNG, BMP, GIF) |
| `video` | Video file (decoded frame-by-frame via ffmpeg) |
| `gif` | Animated GIF |
| `color` | Solid RGB color |
| `sensor` | Live sensor gauge (CPU temp, GPU temp, etc.) |

### Fan Curves

Fan curves map a temperature source (any shell command) to a speed percentage.
Points are linearly interpolated; temperatures outside the curve range clamp to the nearest point's speed.

### Fan Speed Modes

| Mode | Description |
|------|-------------|
| `0` | Off (0% PWM) |
| `"curve-name"` | Follow a named fan curve |
| `1-255` | Constant PWM duty (1=0.4%, 128=50%, 255=100%) |
| `"__mb_sync__"` | Mirror motherboard PWM signal (hardware passthrough) |

## Troubleshooting

**Daemon won't start / no devices found:**
```bash
# Check udev rules are loaded
sudo udevadm test /sys/bus/usb/devices/<your-device>

# Check daemon logs
journalctl --user -u lianli-daemon -f
```

**GUI says "Daemon offline":**
```bash
# Verify daemon is running
systemctl --user status lianli-daemon

# Check socket exists
ls -la $XDG_RUNTIME_DIR/lianli-daemon.sock
```

**Permission denied on USB device:**
```bash
# Re-trigger udev after plugging in device
sudo udevadm trigger
```

## License

MIT. See [LICENSE](LICENSE).

This project is not affiliated with Lian Li Industrial Co., Ltd.
Protocol information was obtained through reverse engineering for interoperability purposes.

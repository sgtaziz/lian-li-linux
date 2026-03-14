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
| Galahad II LCD / Vision AIO | Yes | Yes | 480x480 | Yes | -* |

### Wireless (via TX/RX dongle)

| Device | RGB | LCD | Tested |
|--------|:---:|:---:|:------:|
| UNI FAN TL V2 (LCD / LED) | Yes | 480x480 | Yes |
| UNI FAN SL V3 (LCD / LED) | Yes | 480x480 | - |
| UNI FAN SL-INF | Yes | - | - |
| UNI FAN CL / RL120 | Yes | - | - |

### USB (Standalone)

| Device | LCD | Tested | Notes |
|--------|:---:|:------:|-------|
| HydroShift II LCD Circle | 480x480 | Yes | LCD Supports Background image only |
| Lancool 207 Digital | 1472x720 | Yes | LCD Supports Background image only |
| Universal Screen 8.8" | 1920x480 | - | LCD Supports Background image only |


\* Galahad II LCD / Vision uses the same driver as HydroShift LCD AIO.

If you've tested a device that isn't marked above, please [open an issue or PR](https://github.com/sgtaziz/lian-li-linux/issues) to update this table.

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

Or with any AUR helper (`paru`, `trizen`, etc.). This installs both binaries, udev rules, systemd service (auto-enabled), desktop entry, and icons. After installing, reboot or run:
```bash
sudo udevadm control --reload-rules && sudo udevadm trigger
systemctl --user daemon-reload && systemctl --user start lianli-daemon
```

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
sudo pacman -S hidapi libusb ffmpeg fontconfig mesa libxkbcommon wayland libx11 libinput libdrm clang cmake pkg-config

# Ubuntu / Debian
sudo apt install libhidapi-dev libusb-1.0-0-dev libudev-dev libfontconfig-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libinput-dev libdrm-dev \
  libgl-dev libegl-dev clang cmake pkg-config ffmpeg

# Fedora
sudo dnf install hidapi-devel libusb1-devel fontconfig-devel \
  libxkbcommon-devel wayland-devel libX11-devel libinput-devel libdrm-devel \
  mesa-libGL-devel mesa-libEGL-devel clang cmake pkg-config ffmpeg
```

3) Build:
```bash
cargo build --release
```

Binaries: `target/release/lianli-daemon` and `target/release/lianli-gui`

4) Install udev rules (required for USB access without root):
```bash
sudo cp udev/99-lianli.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

5) Install and start the daemon:
```bash
# Copy binary
cp target/release/lianli-daemon ~/.local/bin/

# Install and start user systemd service
mkdir -p ~/.config/systemd/user
cp systemd/lianli-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now lianli-daemon
```

A default config is created automatically at `~/.config/lianli/config.json` on first run.

6) Install the GUI (optional):
```bash
cp target/release/lianli-gui ~/.local/bin/

# Install icons
for size in 32x32 128x128 256x256; do mkdir -p ~/.local/share/icons/hicolor/$size/apps; done
cp assets/icons/32x32.png ~/.local/share/icons/hicolor/32x32/apps/lianli-gui.png
cp assets/icons/128x128.png ~/.local/share/icons/hicolor/128x128/apps/lianli-gui.png
cp assets/icons/128x128@2x.png ~/.local/share/icons/hicolor/256x256/apps/lianli-gui.png

# Install desktop entry
cp lianli-gui.desktop ~/.local/share/applications/
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

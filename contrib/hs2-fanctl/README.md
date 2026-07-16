# hs2-fanctl — HydroShift II fan controller

A small standalone service that replaces the HydroShift II's **pulsating firmware fan curve**
with a smooth, coolant-temp-driven curve, using the protocol in
[`docs/hydroshift2-fan-protocol.md`](../../docs/hydroshift2-fan-protocol.md).

Directly-connected HydroShift II units currently expose no wired fan device (the HID path is
HID-only), so their fans run the onboard curve — which chases the spiky CPU die temp and
audibly hunts. `hs2-fanctl` instead follows the **coolant** temperature (naturally smooth,
thermal mass of the loop), with an EMA low-pass and a slew-rate limit on top, so fan speed only
ever drifts. It streams `0xFB` continuously to hold the setpoint; if the service stops, the
firmware curve simply resumes (built-in failsafe — the cooler never sticks at a manual speed).

## Build

```sh
cargo build --release -p lianli-devices --bin hs2-fanctl
```

## Install

```sh
sudo install -m755 target/release/hs2-fanctl /usr/local/bin/hs2-fanctl
sudo install -m644 contrib/hs2-fanctl/hs2-fanctl.json /etc/hs2-fanctl.json
sudo install -m644 contrib/hs2-fanctl/hs2-fanctl.service /etc/systemd/system/hs2-fanctl.service
sudo systemctl daemon-reload
sudo systemctl enable --now hs2-fanctl
```

The device is claimed continuously, so stop `hs2-fanctl` before using the LCD.

## Monitor

```sh
systemctl status hs2-fanctl
# Live: coolant temp (smoothed), commanded duty %, per-fan RPM, pump RPM
journalctl -u hs2-fanctl -f -o cat
```

Example line:

```
coolant 43.0C (ema 43.3) -> duty 60% | fans 1312/1316/1310 rpm  pump 2148 rpm
```

## Configure

Edit `/etc/hs2-fanctl.json` and `sudo systemctl restart hs2-fanctl`.

| key | meaning |
|-----|---------|
| `temp_source` | `"coolant"` (recommended) or `"cpu"` (k10temp `Tctl`) |
| `curve` | `[temp_c, duty_pct]` points, linearly interpolated |
| `min_duty_pct` / `max_duty_pct` | hard clamps |
| `smoothing_tau_secs` | EMA time constant on the temp reading (bigger = smoother) |
| `slew_pct_per_sec` | max duty change per second (caps audible ramps) |
| `interval_secs` | poll/stream interval |

For a dead-silent idle, drop the first curve point (e.g. `[38, 20]`). Coolant has high thermal
inertia, so it only crosses ~44 °C under sustained all-core load — the default curve stays quiet
in the normal band and ramps only when the loop is genuinely saturated.

> Note: this is a standalone service, not daemon-integrated. It shares the WinUSB handle with
> the LCD, so run one or the other. Pump-speed control isn't wired (that opcode isn't captured
> yet); the pump stays on its firmware default.

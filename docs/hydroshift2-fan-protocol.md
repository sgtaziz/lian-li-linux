# HydroShift II — pump/fan USB control protocol

Reverse-engineered spec for controlling the **HydroShift II** (`1CBE:A021`) pump/fans over
its WinUSB link, contributed toward [#101](https://github.com/sgtaziz/lian-li-linux/issues/101).
Derived from an L-Connect 3 USBPcap capture on Windows and validated live on Linux against a
standalone probe (offered separately — see *Validation* below). Coolant/RPM readings were
cross-checked against the pump's own onboard LCD readout.

## Transport

Pump/fan control shares the **same WinUSB channel and DES-CBC scheme as the LCD** — no separate
interface, and no wireless dongle involved. Bulk `EP 0x01` OUT / `EP 0x81` IN, 512-byte frames.

Host→device frames are DES-CBC encrypted exactly like the LCD commands
(`key = IV = "slv3tuzx"`, PKCS7 over 500 plaintext bytes → 504 cipher bytes in a 512-byte frame,
trailer `[510]=0xA1 [511]=0x1A`) — i.e. the existing `crypto.rs::build_winusb`. **Device→host
replies are plaintext.**

Plaintext command layout (before encryption):

```
[0]        command byte
[2]=0x1A [3]=0x6D    fixed magic
[4:8]      u32 LE timestamp (ms since a monotonic base; must not go backwards)
[8:]       params
```

## Wake requirement (the non-obvious part)

Once the device has been put into LCD "play" mode it **silently ignores fan commands and never
answers status polls**. A cold open from Windows/L-Connect answered immediately, but coming from
an active LCD session on Linux it did not. A short wake preamble re-arms the command/telemetry
channel:

**`StopPlay 0x7B` → `StopClock 0x34` → `GetVer 0x0A`** (~150 ms apart).

The first command is swallowed; after `StopClock`/`GetVer` the channel is live. (`GetVer`'s
plaintext reply carries the device id string at `[8:]`, e.g. `lianlih2_0004_0020`.)

## Commands

### `0xFA` — Get status (poll)

params: none. Plaintext reply:

```
[8]=0x0A               marker
[9]                    fan count (0x03)
[10],[11],[12]         per-fan configured PROFILE duty %  (nominal, not live-RPM-derived)
[13]                   COOLANT TEMP, whole °C  (verified vs the pump LCD:
                       [13]=0x28 while the LCD read 40 °C, → 0x29 as it reached 41 °C)
[14:16],[16:18],[18:20]  fan RPM ×3  (u16 BE)
[20:22]                pump RPM  (u16 BE)
[22:28]                constant (7c 35 7f 72 ab e1) — id/serial, not telemetry
[30:32]                constant 0x0116 — NOT temperature (do not use; it never tracks)
```

### `0xFB` — Set fan speed

params (12 bytes) + 2-byte CRC:

```
[8:12]   = FF 0F A2 00        fixed header
[12:14]  = u16 BE   host sensor mirror — the temp L-Connect shows on-screen; display-only,
                     any plausible value works
[14],[15],[16] = fan duty ×3, raw 0–255  (all three equal in every capture)
[17:20]  = 00 00 00
[20:22]  = CRC-16/XMODEM (poly 0x1021, init 0x0000, no reflection, no xorout)
           over params[8:20], big-endian
```

Pump control is presumably a sibling opcode / different fixed header — not captured yet (the
reference capture only moved the fans; the pump stayed on its firmware curve).

## Behaviour

A single `0xFB` is applied, then **decays back to the firmware temp-curve within a few seconds**.
To hold a manual speed you must re-stream `0xFB` continuously (L-Connect sends one about every
4 s). When streaming stops, the onboard curve reasserts on its own — a convenient built-in
failsafe: the cooler can never get stuck at a manual setpoint if the host process dies.

## Validated control authority

Live on Linux, streaming `0xFB` at a fixed duty:

| duty (0–255) | fans (RPM) |
|---|---|
| 50  | ~415 |
| firmware idle (~41 %) | ~1400–1600 |
| 255 | ~2150 |

Clean and monotonic across all three fans.

## Validation

Confirmed end-to-end with a small standalone probe built against this workspace's
`lianli-devices`/`lianli-transport` crates (reusing `crypto.rs`'s `build_winusb`): it does the
wake preamble, polls `0xFA`, streams `0xFB` at a target duty, and reads back RPM/coolant. Happy
to contribute the probe binary and a longer-run controller as follow-ups if useful — just say
the word.

## Suggested integration point (defer to your design)

The device shares the WinUSB handle with the LCD, so a fan/pump driver likely wants to live under
the same `WinUsbLcdDevice`/transport rather than the HID `create_wired_controllers` path (which is
HID-only — the reason a directly-connected HydroShift II currently enumerates with zero wired fan
devices). A `FanDevice`-style impl would map `set_fan_speeds` → streamed `0xFB` and
`read_fan_rpm`/pump RPM/coolant → `0xFA`, with the wake preamble in its init.

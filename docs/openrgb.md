# OpenRGB lighting control

Enable the OpenRGB SDK server in RGB or Settings and connect OpenRGB to its configured port (6743 by default). Lian Li Linux continues handling cooling and the LCD. Disable OpenRGB's native detectors for devices already controlled by this daemon.

## AL V2 inner and outer regions

Enable **AL V2 inner/outer regions** and save. Each populated AL V2 fan group appears as one device with Inner and Outer zones in OpenRGB when connected using SDK v6. Select a zone to change its effect independently. Other devices, including the AIO, keep their existing layout.

Static and Breathing support one colour per fan in each region. Other animations run across the region and offer the palettes, speed and direction controls supported by the driver.

**Entire Device** offers effects supported by both regions. Selecting an effect there applies it to both. Selecting a zone effect overrides that region; **Follow Device Mode** restores the device's current effect for that zone. Effects supported by only one region remain available on that zone.

Older SDK clients (v0–5) see separate Inner and Outer devices. Both layouts share the same lighting state.

SDK v6 sends accepted mode and colour changes to all connected v6 clients, including changes made through an older client. Older clients must refresh or reconnect to read changes made elsewhere. The AIO and other ordinary devices also share their last accepted SDK settings across connections; these are not readings of the physical LEDs. Their initial descriptor retains the existing Direct/white defaults until an SDK command updates it.

For ordinary devices, a mode change is attempted on every zone. If some zones reject it, the daemon logs the failures and reports the requested setting for the device; the zones may differ until a later successful write. If every zone rejects it, the reported mode remains unchanged.

## Profiles and reconnecting

Changing this setting reconnects OpenRGB and changes the affected devices' identities. Recreate profiles for those fan groups after changing the setting. Their v6 identity ends in `:regions`; older clients use `:inner` and `:outer`.

Lighting settings survive a client reconnect. SDK commands change the current session without overwriting native saved presets. After restarting the daemon or SDK server, the reported starting state comes from the effective native settings, including active presets. The hardware can retain the previous session's effect until another command is sent. Load an OpenRGB profile to restore that session's lighting.

Changing the available devices or their LED counts disconnects SDK clients so they can reconnect with the updated device list. Devices whose capabilities have not changed retain their session settings and pending writes. A removed fan group starts from native configuration if it is detected again. A reopened group with unchanged capabilities replays its session settings. Unrelated wireless changes do not restart fan effects. This applies with or without the region setting enabled. The server supports protocol v6 for both layouts and negotiates an older version when required.

The bridge keeps the latest complete update for each fan group and sends it through the native group-effect driver. Failed writes receive up to three attempts with increasing delays; sending the same setting again can retry an exhausted update. The first failure and retry exhaustion are logged, including ownership or policy refusals. An SDK state notification reports the accepted setting; it does not confirm that the LEDs have displayed it.

## Configuration and access

The setting is `rgb.openrgb_regions`, disabled by default.

Each connection has a bounded outgoing queue and one socket writer. A client that cannot keep up is disconnected so it can reconnect and read the current state; its socket writes do not block other clients.

The existing SDK listener has no authentication and binds all IPv4 interfaces. Use the host firewall to limit access if it should be local only. Run only one bridge controlling these devices.

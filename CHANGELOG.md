# Changelog

Generated from git history by git-cliff (see .github/cliff.toml).
Regenerate rather than editing by hand.

## v0.8.11 — 2026-09-06

### Fixes
- Fix TL fan and ENE6K77 fan speed assignment  (`184c06b`)

### Other changes
- Fix HID H.264 stalls caused by periodic recovery checks  ([#185](https://github.com/sgtaziz/lian-li-linux/pull/185))
- Add release notes and changelog generation  ([#182](https://github.com/sgtaziz/lian-li-linux/pull/182))

## v0.8.10 — 2026-09-05

### Fixes
- **hydroshift**: Re-apply LCD settings on transport reopen and make stream recovery resilient (fixes [#166](https://github.com/sgtaziz/lian-li-linux/issues/166))  ([#180](https://github.com/sgtaziz/lian-li-linux/pull/180))

### Other changes
- Remove duplicate entries when both wired and wireless is connected  (`6577876`)
- Make sensor dropdown more visible  (`5496c18`)

## v0.8.9 — 2026-09-04

### Fixes
- Answer OpenRGB profile/plugin list requests so SDK clients don't hang  ([#158](https://github.com/sgtaziz/lian-li-linux/pull/158))
- HydroShift II LCD bulk write timeout cancels URBs mid-packet on NAK stalls; log stalls  ([#161](https://github.com/sgtaziz/lian-li-linux/pull/161))
- Fix PKGBUILD for systems using rustup  (`3d80019`)
- Bound ffmpeg/ffprobe runs so a hung helper can't wedge the daemon  ([#175](https://github.com/sgtaziz/lian-li-linux/pull/175))
- Re-bind configured wireless devices that were already unbound at startup  ([#176](https://github.com/sgtaziz/lian-li-linux/pull/176))
- Fix tests  (`5b6af77`)

### Other changes
- Update tested devices  (`9c586ae`)
- AUR is back  (`22d12ca`)
- Fix LCD stream deadlock  ([#154](https://github.com/sgtaziz/lian-li-linux/pull/154))
- Wired HydroShift II: raw 0-255 fan duty, stop blanking the LCD, and fix the shutdown deadlock  ([#152](https://github.com/sgtaziz/lian-li-linux/pull/152))
- Fix EVDI AB24 framebuffer channel order  ([#178](https://github.com/sgtaziz/lian-li-linux/pull/178))
- Refuse wireless bind and unbind when the owning master dongle is online  (`915d138`)
- Move wireless channel off shared frequencies when other master dongles are in range  (`05214cb`)
- Show wireless devices owned by another controller in the gui and gate takeover  (`e2034f1`)
- Text fields in the template editor accept input again  (`6183ffb`)
- Thermal alert restore actually runs, holds during the alert, and invalid clock formats stop panicking  (`1561b5d`)
- Channel arbitration ignores lone dongles and reverts failed switches  (`c471502`)
- Vendor style channel migration with poll loop retargeting and a bind tool channel command  (`71cfd0b`)
- Arbitration needs a foreign master, and fan control restarts when wireless devices appear late  (`45701e7`)

## v0.8.8 — 2026-08-22

### Other changes
- Fix wireless discovery rejecting valid SL v3/v4 device records  ([#157](https://github.com/sgtaziz/lian-li-linux/pull/157))
- Treat empty GetDev responses as retry-worthy  (`99dfc32`)

## v0.8.7 — 2026-08-21

### Features
- Feature parity against old gui for template editor  (`a25de2f`)

### Fixes
- RGB config resync uses stale self.config, silently reverting IPC-driven changes  ([#145](https://github.com/sgtaziz/lian-li-linux/pull/145))
- Direct LED selection not cleared after Apply, overwriting earlier colors  ([#146](https://github.com/sgtaziz/lian-li-linux/pull/146))
- Saved RGB preset can be tagged with a stale (non-Direct) effect  ([#147](https://github.com/sgtaziz/lian-li-linux/pull/147))
- OpenRGB direct colors revert to onboard rainbow after fan PWM updates  ([#149](https://github.com/sgtaziz/lian-li-linux/pull/149))
- Fix pacing for GIF/Video  (`25da93d`)
- Fix wireless devices dropping binding and fans stopping  (`30a53b7`)
- Fix gui bugs  (`dd3255b`)

### Other changes
- Hydroshift & config serial persistance fixes  (`bece34f`)
- Wire up fan/pump control and coolant telemetry for wired AIOs  (`48c6fc6`)
- Pin non-buggy naive-ui version  (`795506e`)

## v0.8.6 — 2026-08-10

### Fixes
- Fix hid matching  (`5adfde1`)

### Other changes
- Note AUR status in README  (`0d079a5`)
- Move udev rule to 60  (`857c5dd`)
- Update sensor labels to be more descriptive  (`0f1bf17`)
- Drop pacakge_lock to unblock copr builds  (`cdb0500`)

## v0.8.5 — 2026-08-10

### Fixes
- Bind Vision 9.2" and Flex LCDs to their configured panel  ([#135](https://github.com/sgtaziz/lian-li-linux/pull/135))
- Fix template installs to fetch assets  (`ebcbe14`)

### Other changes
- Update readme for udev install  (`d29c04b`)
- [TEMPLATE] add HydroShift II OLED Curve  ([#136](https://github.com/sgtaziz/lian-li-linux/pull/136))
- Add test to detect missing lcd backends  (`48607f4`)

## v0.8.4 — 2026-08-09

### Fixes
- Fix galahad2 not initializing due to family mismatch  (`1b62203`)

### Other changes
- Cleanup build script for Arch & Fedora, switch to npm for GUI  (`a3f3322`)
- Add support for system-level service, harden udev rules  (`44fe322`)
- Improve Hydroshift failure detection  (`796458e`)
- Formatting  (`134baa6`)

## v0.8.3 — 2026-08-09

### Fixes
- Fix duplicate entries for Hydroshift 2 in AIO page  (`716c55f`)

### Other changes
- Include ffmpeg  (`a4bb8f8`)
- Remove bad test  (`9b362df`)
- Hydroshift/galahad2 lcd fixes  (`ef0d47c`)
- Formatting  (`b8305b8`)
- Add distrobox steps  (`3277448`)
- Update vendor deps  (`a0ab46d`)

## v0.8.2 — 2026-08-08

### Fixes
- Fix gifs and add testing  (`bd85e14`)

### Other changes
- Move evdi into sep. package for systems that already include it  (`ddfbf45`)
- Fedora packaging updates  (`98f3656`)

## v0.8.1 — 2026-08-08

### Fixes
- Fix LCD solid color UI  (`7e2f6ff`)
- Fix split-nals causing partial screen updates  (`08f935e`)

### Other changes
- Bump ffmpeg-next 8.1 -> 9.0 for FFmpeg 9.0 support  ([#129](https://github.com/sgtaziz/lian-li-linux/pull/129))
- [TEMPLATE] add Perfmon 9.2 landscape system monitor  ([#116](https://github.com/sgtaziz/lian-li-linux/pull/116))
- Unify cargo.toml and upgrade packages  (`7b7f7ad`)
- Add notes for immutable fedora installs  (`4f9e435`)
- Optimize libx264 params  (`77233b8`)

## v0.8.0 — 2026-08-07

### Fixes
- Fix package naming  (`0a5e159`)
- Fix h264 fps pacing  (`d1b6226`)

### Other changes
- Properly respect fps limit in settings, rename for clarity  (`dffe522`)
- Split winusb driver into proper separate drivers per family  (`e4d925c`)
- Add fedora copr packaging  (`83fbcaf`)
- Update fedora copr install instructions  (`bc9093c`)
- Copr require ffmpeg  (`0dca847`)
- Free not non-free  (`63f6b47`)

## v0.7.6 — 2026-08-06

### Other changes
- H264 streaming correctness improvement  (`3d23383`)

### Refactoring
- Refactor hydroshift/galahad2 to avoid double-opening  (`66b5945`)

## v0.7.5 — 2026-08-04

### Other changes
- Readd hidapi backend, rusb too unstable  (`3bd331b`)
- Use static linking for hidapi  (`904f0e3`)
- Dont spawn recovery thread for hs fw < 1.2  (`680974f`)
- Formatting and clippy fixes  (`67800b4`)
- Update udev rules to include hidraw devices  (`d79fdd2`)

## v0.7.4 — 2026-08-02

### Fixes
- LCD template list not refreshed across windows after save  ([#124](https://github.com/sgtaziz/lian-li-linux/pull/124))
- "New" template button opens an existing template instead of blank  ([#123](https://github.com/sgtaziz/lian-li-linux/pull/123))

### Other changes
- Add sensor unit labels  (`03e61e4`)
- Add shared usb transport for hs2 lcd  (`aab47b4`)
- Cargo clippy fixes  (`cdc21b2`)

## v0.7.3 — 2026-08-02

### Fixes
- Stop periodic re-renders from wiping in-progress curve name edits  ([#121](https://github.com/sgtaziz/lian-li-linux/pull/121))
- Pad short fan-speed arrays instead of rejecting them  ([#119](https://github.com/sgtaziz/lian-li-linux/pull/119))
- Fix reset loop on Hydroshift LCD  (`b4e3f0d`)

### Other changes
- Update gui version string to match CARGO_PKG_VERSION  (`c618e77`)
- Improve h264 streaming and playback  (`d12b2f1`)

## v0.7.2 — 2026-08-01

### Other changes
- Add manual PKGBUILD instructions  (`84fe4b1`)
- Prevent overflow during h264 streaming  (`61475bc`)
- Add light mode to gui  (`a371163`)
- Make h264 hw encoding opt-in  (`3850bbe`)

## v0.7.0 — 2026-08-01

### Other changes
- Keep wireless fan PWM refreshed  ([#77](https://github.com/sgtaziz/lian-li-linux/pull/77))
- Add custom gradient stops for radial gauges  ([#82](https://github.com/sgtaziz/lian-li-linux/pull/82))
- Add SetRgbFrames IPC to upload onboard wireless animations  ([#93](https://github.com/sgtaziz/lian-li-linux/pull/93))
- Architecture rewrite  ([#106](https://github.com/sgtaziz/lian-li-linux/pull/106))

## v0.6.1 — 2026-05-23

### Other changes
- Update tested devices  (`25da7c8`)
- [TEMPLATE] add Lancool 207 A5  ([#74](https://github.com/sgtaziz/lian-li-linux/pull/74))
- Fan controller and hydroshift/galahad2 init fixes  (`09c0c63`)
- Set wireless pwm to 0 for stop  (`a06152d`)

## v0.6.0 — 2026-05-15

### Fixes
- Fix TL LCD daisy-chain enumeration, attach, and GUI labeling  (`44a0afc`)

### Other changes
- Oops forgot this guy  (`b6b64d1`)
- Update README Fedora instructions  (`904c3b3`)
- Change defaults for sensor gauge to be actually usable  (`bbc1c8b`)
- Resend frames for TL LCD to keep stream alive  (`7783f2f`)
- Update docker build script and steps  (`735a715`)

## v0.5.13 — 2026-05-12

### Other changes
- Cleanup unused command  (`08730d5`)
- Add configurable hysteresis to fan controller  (`7cb884a`)

## v0.5.12 — 2026-05-11

### Fixes
- **galahad2-trinity**: Register AIO as fan device and fix protocol issues  ([#67](https://github.com/sgtaziz/lian-li-linux/pull/67))

## v0.5.11 — 2026-05-11

### Other changes
- Default lancool 207 to non-h264, opt-in via env  (`f8ce73e`)

## v0.5.10 — 2026-05-09

### Other changes
- Pace h264 streaming by encoded fps  (`1fa8648`)
- Readd args to reduce h264 latency  (`e6af959`)

## v0.5.9 — 2026-05-08

### Fixes
- Fix GIF/Videos by restoring pix_fmt arg  (`e86f418`)

### Other changes
- Remove custom image upload for AIO  (`f86a223`)
- Send wireless AIO param block unconditionally every second  (`33cf934`)

## v0.5.8 — 2026-05-08

### Other changes
- Resend wireless RGB on device firmware drift  (`61a6a62`)

## v0.5.7 — 2026-05-08

### Other changes
- Reconnect wireless on disconnect  (`4b276c0`)
- Move LCD health check to per-device thread  (`07f5912`)
- Recreate media runtime on AIO LCD reset  (`6014520`)
- Drop wireless TX after 5 consecutive send failures  (`6b7949c`)
- Change args for h264 transcode + frame extraction  (`be34a5b`)
- Add wired device hot-plug detection  (`b6e041d`)

## v0.5.6 — 2026-05-07

### Fixes
- Fix ene6k77 mode bytes, rpm, zone count, fan speeds, and quantity keepalive  (`69f8e0e`)

### Other changes
- Send master-clock heartbeat (RF 0x14) to prevent fan RPM spikes  ([#61](https://github.com/sgtaziz/lian-li-linux/pull/61))
- Apply aio changes on save  (`083c71a`)
- Remove device_rotation, set Lancool207 to portrait by default  (`514197f`)
- Reload daemon after system sleep  (`45254f3`)

## v0.5.5 — 2026-05-06

### Fixes
- Fix HS streaming with NAL alignment + recovery loop  (`70fc153`)

### Other changes
- Update README.md  (`28d5742`)
- Display mode switcher hotfix  (`11b84c1`)
- Delay rgb controller for wireless devices  (`9f1dacf`)

## v0.5.3 — 2026-05-02

### Fixes
- Fix AIO LCD blank screen and C-Command reliability  (`1b7c5e6`)

### Other changes
- Run cargo fmt  (`fef402a`)
- Fallback to next encoder when ffmpeg fails  (`89f8baf`)
- Add h264 render path for AIO LCDs  (`3db77c0`)

## v0.5.2 — 2026-05-01

### Fixes
- Fix artifacts on h264  (`2322f32`)

## v0.5.1 — 2026-05-01

### Other changes
- Add h264 render path for supported devices in Custom mode  (`b1c6fa3`)
- Optimize custom mode by using a faster overlay function  (`62075dc`)
- Drop per-frame scratch clone in custom  (`f6bd187`)
- ENE/SL fan fixes, ability to configure fans per port  (`d100b23`)

## v0.5.0 — 2026-04-30

### Other changes
- Add smooth edges toggle for custom mode  (`abd0f77`)
- Cache rendered widget output  (`2d71b12`)
- Reuse scratch frame buffer  (`c346394`)
- Upgrade image-related crates and use SIMD resize  (`6552b1b`)
- Hwaccel on decode and fix widget fps  (`53b7fe6`)

### Refactoring
- Refactor and cleanup crates  (`b2789fc`)

## v0.4.8 — 2026-04-25

### Other changes
- Drop old transport before reopen to avoid claiming against ourselves  (`cec73ab`)

## v0.4.7 — 2026-04-25

### Other changes
- Run cargo fmt  (`75f495f`)
- Drop tl lcd fan capability tag  (`b78bb32`)
- Rename TL LCD device entry in GUI  (`4868397`)
- Respect fps config for gifs  (`3db11a7`)
- Add mb_sync support for galahad 2 trinity  (`5295b15`)
- Clamp galahad 2 trinity fan pwm  (`580f8b3`)
- Refresh RPM via handshake for galahad 2  (`4983637`)
- Add missing rgb mode variants  (`6e050ef`)
- Add device-specific mode-byte wrappers and missing rgb modes  (`94be98a`)
- Use proper read timeouts for various devices  (`7f72ff3`)
- Add disabled flag to RgbEffect  (`c4f8366`)
- Claim devices continuously for rusb and rusb_hid  (`2e558d9`)
- Add retry path for reads  (`49292c6`)
- Warn when interface has multiple interrupt endpoints  (`6850392`)
- Factor out shared open_with_retry helper  (`db9e090`)
- Skip USB reset on startup if device responds to descriptors  (`fd747e6`)
- Pid-lock daemon to enforce single instance  (`7332d0c`)
- Bind HID and poll for hidraw before reset  (`c0bdf83`)
- Drop set_alternate_setting  (`06b78b9`)

## v0.4.5 — 2026-04-24

### Other changes
- Less aggressive retries - only reset usb as last resort  (`050071f`)

## v0.4.4 — 2026-04-24

### Fixes
- Fix rusb_hid bugs and set an exit timeout  (`774e1eb`)

## v0.4.3 — 2026-04-24

### Other changes
- Add new neon us88 template  (`4c383c4`)
- Claim HID interfaces before probe in rusb_hid driver  (`b942dbf`)

## v0.4.2 — 2026-04-24

### Fixes
- Fix transparency with webm and apng and nvidia-smi stutter  (`a10656e`)

### Other changes
- Add new sensor categories  (`be6c070`)
- Resolve font paths for templates  (`b74aa0b`)

## v0.4.1 — 2026-04-24

### Other changes
- More robust wireless bind/unbind with GUI fixes  (`d76cc3f`)
- Readd AIO devices to RGB page  (`38f9712`)

## v0.4.0 — 2026-04-23

### Other changes
- Use tags instead of latest commit  (`80e5e18`)
- Add US8.8 template by hanzzz2909  (`c136cac`)
- Lock slint rev to known version  (`f3b8385`)
- Update README build steps and tested devices  (`d4005c6`)
- Add initial support for wireless AIO (H2)  (`17c0f40`)
- Hide Pump tag on H2 wired device, wireless should handle it  (`9918725`)
- Make custom LCD type respect update interval of individual widgets  (`a96cb84`)
- Add 2x supersampling for widgets that need it  (`7108aa5`)
- Add network and disk sensors  (`47b9168`)

## v0.3.6 — 2026-04-17

### Build
- Build latest hidapi for github workflow  (`af69f85`)

### Features
- Support for building the project with docker  (`b9efed4`)
- Feature-parity with Tauri/Vue GUI  (`1fbad15`)
- Add HydroShift II LCD Square (1cbe:a034) support  ([#16](https://github.com/sgtaziz/lian-li-linux/pull/16))
- **rgb**: Add per-LED color control and preset save/load  ([#36](https://github.com/sgtaziz/lian-li-linux/pull/36))

### Fixes
- Fix TL Fan per-fan RGB control and OpenRGB apply-all going black  (`0b118cc`)
- Fix wireless fan PWM, correct byte[1] to 0x10 and sequence index to 1 for now  (`9e11631`)
- Fix TL Fan side mode, split group setup and route scoped static to group light  (`22c16b8`)
- Fix ENE device names to show specific model variant instead of generic SL/AL Controller  (`6a8640d`)
- Fix config race condition causing empty config in UI  (`df67fd6`)
- Fix new Slint UI bugs  (`3b91c02`)
- Fix missing imports for build/test  (`0a0807a`)
- Fix WinUSB LCD init sequence, add portrait rotation, and read flush  (`ca480b3`)
- Force Slint winit backend to fix Arch/AUR build failure  ([#8](https://github.com/sgtaziz/lian-li-linux/pull/8))
- Fix ENE 6K77 RGB commit, color report type, and SL Infinity encoding  (`764e469`)
- Fix HydroShift init timeout and firmware version parsing for comma-separated strings  (`f4529a8`)
- Fix strimer LED count mapping  (`3faef8b`)
- Fix RGB zone validation rejecting pump head on fanless AIO  (`cf07736`)
- Fix display mode switch by adding Reboot command  (`edb2698`)
- Fix bug causing LCD initial frame to never be sent  (`bf99a06`)
- Fix RGB zone validation rejecting Strimer  (`07a16ce`)
- Fix strimer LED count  (`6fcad85`)
- Fix packet timestamp to use milliseconds since start  (`e770fc7`)
- Fix Universal Screen resolution to 480x1920 portrait (rotated 90deg)  (`270b86a`)
- Fix H264 streaming stalls and content switch deadlocks  (`d359cb0`)
- Fix orientation for LCDs by swapping render dimensions before rotation  (`2555bd8`)
- Fix and polish Custom LCD template system  (`42040bc`)
- Fix resolution badge in online browser  (`da60dec`)
- Fix README manual install steps  (`fd525c7`)
- Fix handshake for devices with two-read firmwares, add Hydroshift reset support  (`4635dfc`)
- Fix h264 orientation logic  (`0537df6`)
- Fix LCD on TL LCD fans (wire up and unique serial fix)  (`1a83c9c`)
- Fix static images on TL LCD  (`82a9e68`)
- Fix build workflow  (`59fa6a7`)
- Fix install for rustup users  (`71acdfe`)
- Fix duplicate LCD devices in GUI  (`819fda2`)
- Fix PWM slider not live-updating label  (`4ccfc44`)
- Fix deps for evdi  (`44d30a3`)
- Fix pkgbuild  (`246b333`)
- Fix fragment_stream_a panic on payloads sized  (`c979ca7`)
- Fixes for h264 LCD streaming, add hw accel support (can be disabled with LIANLI_DISABLE_HW_VIDEO=1)  (`1ee4b97`)

### Other changes
- First commit  (`0594895`)
- Add RGB shared types, RgbDevice trait, and IPC/config foundation  (`c9a84b5`)
- Implement RgbDevice for TL Fan, ENE 6K77, and Galahad2 Trinity with IPC stubs  (`e9c3fa1`)
- Add tinyuz compression and wireless RGB streaming via RF protocol  (`a5fcf0f`)
- Integrate RGB controller into daemon service loop and IPC handlers  (`f8d35c3`)
- Add motherboard ARGB sync support for TL, ENE 6K77, and Galahad2 devices  (`d8889ce`)
- Per-port RGB devices, fix per-fan lighting, unique device IDs, OpenRGB stability fixes, GUI improvements  (`46b5f4d`)
- Configurable OpenRGB server port, live status reporting, skip native RGB when OpenRGB is enabled  (`e3ad871`)
- Reflect new features/requirements in README  (`1a3087f`)
- Async rgb writer thread with frame dropping, 1x header for streaming  (`e56ad8f`)
- Reduce OpenRGB streaming lag and USB polling overhead  (`4932d8b`)
- Add RGB support for AIO LCD devices (HydroShift LCD, Galahad2 LCD/Vision)  (`cd22b59`)
- Add devices LED scopes and scope selector GUI  (`7f78fec`)
- Wire TL Fan direction commands (swap_lr/swap_tb) with GUI toggles and config persistence  (`d749cca`)
- Add synced indicator on TL Fan zones when group light is active  (`a488166`)
- Add fan PWM MB Sync support for wireless devices that support it  (`1d4fd71`)
- Remove config file hot-reload, only reload on IPC trigger from GUI  (`b27d914`)
- Change systemctl command in daemon-not-running banner to use user service  (`7083825`)
- Update device capabilities  (`313d122`)
- Use WEBKIT_DISABLE_DMABUF_RENDERER to fix tauri/NVIDIA  (`acbf2ea`)
- Add rust build test  (`384fc4c`)
- Cleanup comments  (`bb46a5e`)
- Install systems deps for tauri  (`5e31b79`)
- Checkout submodules for tests  (`518c92c`)
- Add bun install step  (`85904ad`)
- Initial slint GUI, WIP  (`7427d0d`)
- Merge pull request #2 from ealcantara22/build-with-docker  (`e75cce8`)
- Replace Tauri/Vue GUI with pure Rust Slint GUI  (`5d9c01c`)
- Add arch pkgbuild  (`7440763`)
- Update Arch install instructions to use AUR package  (`1ee54b2`)
- Remove dark/light mode toggle in slint  (`759f6b5`)
- Correct HydroShift II LCD Circle USB PID from 0xa001 to 0xa021  (`7fc3fb1`)
- Add Hydroshift II-specific packet builder format  (`27290af`)
- Use Hydroshift II packet format and remove rotation init  (`d631b71`)
- Route HydroShift II to WinUsbLcdDevice and fix 480x480 media resolution  (`1246134`)
- Avoid double-initializing wired fan devices on startup  (`c5189f4`)
- Add LcdBackend::HidLcd variant  (`4db0ae3`)
- Add HID LCD devices into streaming pipeline and fix screen resolution mapping  (`bdbb2a7`)
- Expose HID LCD devices in IPC device cache with proper hidapi serials  (`cdc2a0a`)
- Add 2-second init delay and use LCDSetting mode for brightness/rotation  (`1a0a7f4`)
- Use 512-byte C-command packets for LCD frames on firmware >= 1.2  (`2021b06`)
- Skip invalid LCD config entries with warnings  (`3322082`)
- Do not remove invalid config entries to allow GUI to see them  (`b959ecf`)
- Reload GUI on daemon reconnect  (`6ac9c15`)
- Add hidraw udev rules  (`c8d89f9`)
- Add more debug logging for lcd candidates  (`33c77b8`)
- Discover HID LCDs via rusb and open by VID/PID for streaming  (`22e7a08`)
- Bypass hidapi with rusb interrupt transport for HID LCD devices  (`2d81396`)
- Use larger buffer to avoid overflow on hid reads  (`e5a5e48`)
- Add better logging for hydroshift LCD  (`785cad1`)
- Bind HID devices through udev rules  (`4271916`)
- Reset USB of HID devices  (`be87f40`)
- Add HID driver selector, shared device handles, rework device IDs, and USB retry logic  (`f5fa189`)
- Auto-detect USB endpoint types  (`5c9a94d`)
- Log LCD read responses and auto-detect endpoint transfer type on init  (`d7b44cd`)
- Default RGB colors to black when missing from config  (`ed0451e`)
- Retry static LCD frames and prefer interrupt on reads  (`02c7f90`)
- Regenerate lock file & add missing depends in PKGBUILD  (`801b1cd`)
- Update README to reflect tested devices & cleanup  (`dadefbc`)
- Cleanup table  (`c8115f4`)
- Disable LTO in PKGBUILD  (`c978289`)
- Link against libhidapi-hidraw  (`ec8a512`)
- Switch to statically linked hidapi  (`62a4a5a`)
- Properly set default fan config to populate UI  (`639243f`)
- Filter wireless devices my master MAC to avoid controlling non-bound devices  (`8319857`)
- Add OpenRGB note in UI  (`f8a960b`)
- Add debug logging for ene6k77  (`d57d32c`)
- Add inner/outer RgbScope support for ene6k77  (`538c079`)
- Skip LCD Application mode command before first frame  (`11681a1`)
- Guard against double init  (`1da44cb`)
- Register HydroShift as fandevice  (`a67833d`)
- Set ENE 6K77 default fan quantity (3 per group) on init  (`54ffb64`)
- Expand ENE 6K77 color data to 36 entries  (`f3686e1`)
- Send correct LED count per ring for dual-ring ENE models  (`5fba204`)
- Treat each group as sep device for ene6k77  (`df7199a`)
- Add mb rpm sync for ene6k77  (`29a915a`)
- Use input report for reading firmware in ene6k77  (`8df8a51`)
- Increase read_input buffer  (`3fd744c`)
- Add wireless device types: WaterBlock, Strimer, LC217, Led88  (`830f21e`)
- Add Strimer Plus to known devices and display switcher PIDs  (`a1542cb`)
- Detect desktop-mode LCD devices and add display mode switch button  (`c3dde7a`)
- Add hydroshift II square desktop mode PID  (`c263e37`)
- Add wireless TX/RX v2 dongle support  (`4119f11`)
- Add V150 wireless device type  (`8555d52`)
- Add universal screen 8.8" lighting device  (`49d0ce6`)
- Separate pump speed control from fan slots for AIO devices  (`4083edb`)
- Update README with new devices  (`a09e28b`)
- Log unbound wireless devices on discovery  (`18a579f`)
- Add wireless device binding via GUI button  (`d58e07a`)
- Implemented event driven main loop for sending frame buffers  ([#12](https://github.com/sgtaziz/lian-li-linux/pull/12))
- Replace expect panics with error returns in media asset preparation  (`e14e775`)
- Initialize required fields when switching LCD media type  (`723f025`)
- Retry wireless polling on error instead of terminating thread  (`56a53e4`)
- Smooth fan curve temperatures and reject out-of-range readings  (`8625f35`)
- Clear LCD layers on init  (`8baa701`)
- Add hwmon sensor support for fan curves and LCD  (`12128fe`)
- Add hwmon sensor dropdown in fan curves  (`87ac7f6`)
- Add hwmon sensor dropdown in LCD sensor gauge  (`cb03c8d`)
- Refresh ui on sensor source change  (`b5b83e6`)
- Use short timeout for LCD frame ack reads to prevent blocking event loop  (`ae036bd`)
- Expose wireless AIO coolant temperature as sensor source  (`011e06f`)
- Decrease default fan update interval  (`0beabd5`)
- Add more logs for out-of-range zones  (`52acd39`)
- Update README with new tested devices  (`03279c2`)
- Remove outdated note from README  (`8a21295`)
- Increase fan update default to 500ms  (`d780708`)
- Detach kernel driver after USB reset  (`25d49e3`)
- Reuse LCD transport on config change  (`3f91d93`)
- Adjust WinUSB USB timeouts  (`6e0c709`)
- Add WinUSB write retry with transport reset on failure  (`f3a4358`)
- Add QueryBlock flow control to avoid buffer overflow  (`c9cf862`)
- Track consecutive USB errors for LCD targets  (`1124426`)
- Add SyncClock to WinUSB LCD init sequence  (`fc2a54e`)
- Move WinUSB LCD frame sending to dedicated thread  (`38cdcac`)
- Add H264 chunked streaming for WinUSB LCD  (`b78b042`)
- Don't reinit on missing frame ack  (`f3ee011`)
- Update README with new tested devices  (`60c1ae5`)
- Sensors reworked  ([#30](https://github.com/sgtaziz/lian-li-linux/pull/30))
- Configurable sensorgauge background image added  ([#31](https://github.com/sgtaziz/lian-li-linux/pull/31))
- Added Doublegauge and Cooler  ([#32](https://github.com/sgtaziz/lian-li-linux/pull/32))
- Cargo fmt across the whole workspace  (`97cdd24`)
- Default gauge_2 to 0..100 so the temp needle uses the full arc  (`c5cc77d`)
- Unify update_interval_ms field for all sensor-driven LCDs  (`5ee3cb5`)
- Draw cooler core separator strip per-frame so it tracks live core count  (`6da93ae`)
- Add MediaType::Custom scaffolding + template data mode  (`98e5ee9`)
- Implement CustomAsset widget renderer + default templates  (`e2159ea`)
- Wire Custom template picker + management into LCD page  (`25e0c2f`)
- Add template layout editor window with drag canvas + live preview  (`2fdb6ec`)
- Retire doublegauge/cooler media types in favor of Custom  (`cc19d58`)
- Change font selection to a drop-down of system fonts  (`b3b7a2e`)
- Show bg color selector for custom LCD & set default to black  (`409f156`)
- Size pre-rotation canvas by orientation so rotated templates render  (`af68afd`)
- Add ability to assign widget render order  (`fc8eeb5`)
- Add render-preview binary that generates preview.png from a template  (`3864426`)
- Recover from LCD write errors with clear_halt before reset  (`e2870bf`)
- Match numeric-prefixed "Custom command" label  (`757c92d`)
- Expose nvidia GPU temp and usage as separate labeled sensor  (`98680f9`)
- Add SensorCategory hint on Widget and resolver  (`f888f5c`)
- Port cooler and doublegauge to the repo templates folder  (`4fdae84`)
- Add template catalog manifest and fetcher/installer  (`13aaaa9`)
- Add online template browser window  (`9c78758`)
- Support AMD GPU usage, file:// catalog URLs, Copy JSON in editor, version 0.3.3  (`1174495`)
- Retire default builtin templates in favor of online templates  (`b7401d3`)
- Add xdg-app-id for proper wayland icon support  (`74bc266`)
- Remove always-on-top flag  (`4d13037`)
- Rename desktop file and installed icons  (`11ba415`)
- Hold device lock for entire fan speed writes to avoid corruption by rgb writes  (`ec1e4b6`)
- Increase default fan update interval to 1000ms  (`1d3fd53`)
- More robust firmware version query for ENE devices  (`41157f0`)
- Prevent fan tick overlap  (`cf57b76`)
- Skip wireless PWM sends when device-reported values are within threshold  (`cdfff59`)
- Poll wireless RX at 1000ms intervals  (`bb8f2ed`)
- Update HDiffPatch  (`04d66f8`)
- Wire presets and per-LED colors to GUI  (`03f8314`)
- Add retry logic on wireless controller if unexpected response  (`cd19f32`)
- Use raw dimensions for h264 (ffmpeg handles rotation)  (`00314c7`)
- Improve wireless stability  (`cb33d80`)
- Improve MB Sync functionality for Wireless devices with PWM header selection  (`f4b9cb0`)
- Loop read buffer on hydroshift until response matches expected  (`fe81ca0`)
- Add additional configuration for existing widgets  (`37648ca`)
- Add letter spacing to Label and ValueText widgets  (`9af6ecb`)
- Add ClockDigital and ClockAnalog widgets  (`f94ce75`)
- Add per-template thumbnail previews to LCD-page dropdown  (`08a2a0e`)
- Auto-increment New Template name on collision  (`43fb64a`)
- Split editor.rs into submodules  (`bfd8403`)
- Use turbojpeg for jpeg encoding  (`5b92d2d`)
- Add nasm and libjpeg-turbo pkgbuild  (`a583f93`)
- Split template_editor.slint into subfiles  (`6958125`)
- Sparkline axis labels, gridlines, range-colored fill, and format parser fixes  (`0f01162`)
- Auto-recover stale USB handles  (`9ebdafb`)
- Add support for Universal Screen 8.8" LED Ring RGB control  (`b1e69de`)
- Add support for Desktop-Mode devices using evdi  (`9b0fac6`)
- Update README with new features  (`b996f40`)
- Move systemd unit from user to templated system service  (`2ea8bde`)
- Add one-short service to setup evdi card  (`e5fae4e`)
- Fixed set fan speed and pump pwm errors, small improvements  ([#41](https://github.com/sgtaziz/lian-li-linux/pull/41))
- Suppress mode-switch warn spam on Desktop/LCD transition  (`f66056f`)
- Schedule eager USB/device refresh after mode switch  (`f250d5f`)
- Cleanup format  (`425da6f`)

### Performance
- Perform USB device reset test  (`496459d`)

### Refactoring
- Refactor custom to make future widgets easier to add  (`c326a3f`)

### Reverts
- Revert daemon to user service, udev-manage evdi access  (`bd52289`)


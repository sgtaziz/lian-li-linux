//! Safe Rust wrapper around libevdi 1.14 (DisplayLink's virtual-DRM library).
//!
//! Scoped to what we need to drive a TURZX USB display: open/add a device
//! node, connect with an EDID, receive mode + update events, grab pixels
//! into a registered buffer, disconnect and close on Drop.
//!
//! Not thread-safe. One `EvdiHandle` per worker thread.

mod ffi;

use anyhow::{anyhow, bail, Result};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::os::raw::{c_int, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, trace, warn};

/// Tracks whether we've ever called `evdi_add_device` successfully so the
/// daemon can ask the kernel to remove all evdi nodes on shutdown.
static ADDED_ANY_DEVICE: AtomicBool = AtomicBool::new(false);

pub use ffi::evdi_mode;

pub const MAX_DIRTY_RECTS: usize = 16;

#[derive(Debug, Error)]
pub enum EvdiError {
    #[error("no evdi device available and evdi_add_device failed")]
    NoDeviceAvailable,
    #[error("evdi_open returned null for device {0}")]
    OpenFailed(c_int),
    #[error("eventfd poll failed: {0}")]
    Poll(std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl From<ffi::evdi_rect> for Rect {
    fn from(r: ffi::evdi_rect) -> Self {
        Self {
            x1: r.x1,
            y1: r.y1,
            x2: r.x2,
            y2: r.y2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh_hz: i32,
    pub bits_per_pixel: i32,
    pub pixel_format: u32,
}

impl From<ffi::evdi_mode> for Mode {
    fn from(m: ffi::evdi_mode) -> Self {
        Self {
            width: m.width,
            height: m.height,
            refresh_hz: m.refresh_rate,
            bits_per_pixel: m.bits_per_pixel,
            pixel_format: m.pixel_format,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    ModeChanged(Mode),
    UpdateReady(i32),
    DpmsChanged(i32),
    CrtcStateChanged(i32),
}

#[derive(Default)]
struct EventSink {
    queue: VecDeque<Event>,
    // Raw handle used by callbacks (e.g. DDCCI) that need to call back into
    // libevdi. Populated at `connect()` time, cleared on disconnect/drop.
    handle_raw: ffi::evdi_handle,
}

extern "C" fn on_dpms(dpms_mode: c_int, user_data: *mut c_void) {
    trace!("on_dpms cb fired: mode={dpms_mode}");
    unsafe {
        if let Some(sink) = (user_data as *mut EventSink).as_mut() {
            sink.queue.push_back(Event::DpmsChanged(dpms_mode));
        }
    }
}

extern "C" fn on_mode(mode: ffi::evdi_mode, user_data: *mut c_void) {
    trace!(
        "on_mode cb fired: {}x{} @ {} Hz bpp={} fmt={:#x}",
        mode.width,
        mode.height,
        mode.refresh_rate,
        mode.bits_per_pixel,
        mode.pixel_format
    );
    unsafe {
        if let Some(sink) = (user_data as *mut EventSink).as_mut() {
            sink.queue.push_back(Event::ModeChanged(mode.into()));
        }
    }
}

extern "C" fn on_update_ready(buffer_id: c_int, user_data: *mut c_void) {
    trace!("on_update_ready cb fired: buffer_id={buffer_id}");
    unsafe {
        if let Some(sink) = (user_data as *mut EventSink).as_mut() {
            sink.queue.push_back(Event::UpdateReady(buffer_id));
        }
    }
}

extern "C" fn on_crtc(state: c_int, user_data: *mut c_void) {
    trace!("on_crtc cb fired: state={state}");
    unsafe {
        if let Some(sink) = (user_data as *mut EventSink).as_mut() {
            sink.queue.push_back(Event::CrtcStateChanged(state));
        }
    }
}

extern "C" fn on_cursor_set(_c: ffi::evdi_cursor_set, _user_data: *mut c_void) {
    trace!("on_cursor_set cb fired");
}

extern "C" fn on_cursor_move(_m: ffi::evdi_cursor_move, _user_data: *mut c_void) {
    trace!("on_cursor_move cb fired");
}

extern "C" fn on_ddcci(data: ffi::evdi_ddcci_data, user_data: *mut c_void) {
    trace!(
        "on_ddcci cb fired: address={:#06x} flags={:#06x} buffer_length={}",
        data.address,
        data.flags,
        data.buffer_length
    );
    unsafe {
        let Some(sink) = (user_data as *mut EventSink).as_mut() else {
            return;
        };
        let handle = sink.handle_raw;
        if handle.is_null() {
            return;
        }
        // evdi's kernel driver validates that the response buffer length
        // matches the request's. Echo the length back with zeroed contents
        // and result=false ("no DDC/CI data available").
        let len = data.buffer_length;
        let zeros = vec![0u8; len as usize];
        ffi::evdi_ddcci_response(handle, zeros.as_ptr(), len, false);
    }
}

pub struct EvdiHandle {
    raw: NonNull<ffi::evdi_device_context>,
    sink: Box<EventSink>,
    ctx: Box<ffi::evdi_event_context>,
    connected: bool,
    cached_event_fd: Option<c_int>,
    _not_send: PhantomData<*mut ()>,
}

impl EvdiHandle {
    pub fn lib_version() -> (i32, i32, i32) {
        let mut v = ffi::evdi_lib_version {
            version_major: 0,
            version_minor: 0,
            version_patchlevel: 0,
        };
        unsafe { ffi::evdi_get_lib_version(&mut v) };
        (v.version_major, v.version_minor, v.version_patchlevel)
    }

    /// Open the first AVAILABLE evdi device, creating a new node if none exist.
    pub fn open_or_add() -> Result<Self> {
        for n in 0..16 {
            let status = unsafe { ffi::evdi_check_device(n) };
            if status == ffi::AVAILABLE {
                return Self::open(n);
            }
        }
        let added = unsafe { ffi::evdi_add_device() };
        if added <= 0 {
            return Err(explain_add_failure().into());
        }
        ADDED_ANY_DEVICE.store(true, Ordering::SeqCst);
        for n in 0..16 {
            let status = unsafe { ffi::evdi_check_device(n) };
            if status == ffi::AVAILABLE {
                return Self::open(n);
            }
        }
        Err(EvdiError::NoDeviceAvailable.into())
    }

    pub fn open(device: c_int) -> Result<Self> {
        let raw = unsafe { ffi::evdi_open(device) };
        let raw = NonNull::new(raw).ok_or(EvdiError::OpenFailed(device))?;
        let mut sink = Box::new(EventSink::default());
        sink.handle_raw = raw.as_ptr();
        let ctx = Box::new(ffi::evdi_event_context {
            dpms_handler: Some(on_dpms),
            mode_changed_handler: Some(on_mode),
            update_ready_handler: Some(on_update_ready),
            crtc_state_handler: Some(on_crtc),
            cursor_set_handler: Some(on_cursor_set),
            cursor_move_handler: Some(on_cursor_move),
            ddcci_data_handler: Some(on_ddcci),
            user_data: sink.as_mut() as *mut EventSink as *mut c_void,
        });
        Ok(Self {
            raw,
            sink,
            ctx,
            connected: false,
            cached_event_fd: None,
            _not_send: PhantomData,
        })
    }

    pub fn connect(&mut self, edid: &[u8], sku_area_limit: u32) -> Result<()> {
        self.connect_with_rate(edid, sku_area_limit, 0)
    }

    /// `evdi_connect2` variant: includes a pixel-per-second throughput hint.
    /// Pass 0 to let the kernel pick a default. DisplayLinkManager uses this
    /// form, and some compositors gate on the hint being present.
    pub fn connect_with_rate(
        &mut self,
        edid: &[u8],
        pixel_area_limit: u32,
        pixel_per_second_limit: u32,
    ) -> Result<()> {
        if edid.is_empty() {
            bail!("connect: EDID must be non-empty");
        }
        unsafe {
            ffi::evdi_connect2(
                self.raw.as_ptr(),
                edid.as_ptr(),
                edid.len() as u32,
                pixel_area_limit,
                pixel_per_second_limit,
            );
            // Leave cursor events disabled — we don't render cursors to
            // the TURZX panel, and advertising HW cursor support via the
            // event channel makes Hyprland gate mode-set on a cursor plane
            // it can't actually use (shows up as `hw cursor` in
            // `tearingBlockedBy`).
        }
        self.connected = true;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if self.connected {
            unsafe { ffi::evdi_disconnect(self.raw.as_ptr()) };
            self.connected = false;
        }
        self.sink.handle_raw = std::ptr::null_mut();
    }

    pub fn register_buffer(&mut self, buf: &mut EvdiBuffer) {
        let raw = ffi::evdi_buffer {
            id: buf.id,
            buffer: buf.pixels.as_mut_ptr() as *mut c_void,
            width: buf.width,
            height: buf.height,
            stride: buf.stride,
            rects: buf.rects.as_mut_ptr(),
            rect_count: MAX_DIRTY_RECTS as c_int,
        };
        unsafe { ffi::evdi_register_buffer(self.raw.as_ptr(), raw) };
        buf.registered_by = Some(self.raw.as_ptr() as usize);
    }

    pub fn unregister_buffer(&mut self, buf: &mut EvdiBuffer) {
        if buf.registered_by == Some(self.raw.as_ptr() as usize) {
            unsafe { ffi::evdi_unregister_buffer(self.raw.as_ptr(), buf.id) };
            buf.registered_by = None;
        }
    }

    /// Returns true when evdi has an update ready for the buffer immediately
    /// (no need to await the eventfd before calling `grab_pixels`).
    pub fn request_update(&mut self, buffer_id: i32) -> bool {
        unsafe { ffi::evdi_request_update(self.raw.as_ptr(), buffer_id) }
    }

    pub fn grab_pixels(&mut self) -> Vec<Rect> {
        let mut rects = [ffi::evdi_rect::default(); MAX_DIRTY_RECTS];
        let mut n: c_int = MAX_DIRTY_RECTS as c_int;
        unsafe {
            ffi::evdi_grab_pixels(self.raw.as_ptr(), rects.as_mut_ptr(), &mut n);
        }
        let n = n.max(0) as usize;
        rects.iter().take(n).copied().map(Rect::from).collect()
    }

    /// Block on the event fd for up to `timeout`, then dispatch any pending
    /// events through the registered callbacks. Drains and returns whatever
    /// accumulated in the event sink.
    pub fn poll_events(&mut self, timeout: Duration) -> Result<Vec<Event>> {
        let fd = if let Some(cached) = self.cached_event_fd {
            cached
        } else {
            let fd = unsafe { ffi::evdi_get_event_ready(self.raw.as_ptr()) };
            if fd < 0 {
                bail!("evdi_get_event_ready returned {fd}");
            }
            trace!("evdi event fd = {fd}");
            self.cached_event_fd = Some(fd);
            fd
        };
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, millis) };
        if rc < 0 {
            return Err(EvdiError::Poll(std::io::Error::last_os_error()).into());
        }
        if rc > 0 && (pfd.revents & libc::POLLIN) != 0 {
            trace!("evdi eventfd POLLIN — dispatching");
            unsafe {
                ffi::evdi_handle_events(self.raw.as_ptr(), self.ctx.as_mut());
            }
        }
        Ok(self.drain_events())
    }

    fn drain_events(&mut self) -> Vec<Event> {
        self.sink.queue.drain(..).collect()
    }

    pub fn raw_event_fd(&mut self) -> c_int {
        unsafe { ffi::evdi_get_event_ready(self.raw.as_ptr()) }
    }
}

/// Best-effort teardown of every evdi card this process created. Requires
/// root to write `/sys/devices/evdi/remove_all`; logs a hint and returns
/// `Ok(false)` when not permitted.
pub fn remove_all_devices() -> Result<bool> {
    if !ADDED_ANY_DEVICE.load(Ordering::SeqCst) {
        return Ok(false);
    }
    const PATH: &str = "/sys/devices/evdi/remove_all";
    match std::fs::OpenOptions::new().write(true).open(PATH) {
        Ok(mut f) => {
            use std::io::Write;
            match f.write_all(b"1") {
                Ok(()) => {
                    debug!("wrote '1' to {PATH} — all evdi nodes removed");
                    Ok(true)
                }
                Err(e) => {
                    warn!("writing to {PATH} failed: {e}");
                    Ok(false)
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            warn!(
                "could not remove evdi nodes on shutdown: {PATH} is root-only. \
                 Run `sudo sh -c 'echo 1 > {PATH}'` manually if you need to \
                 free them before next boot"
            );
            Ok(false)
        }
        Err(e) => {
            warn!("opening {PATH} failed: {e}");
            Ok(false)
        }
    }
}

impl Drop for EvdiHandle {
    fn drop(&mut self) {
        self.disconnect();
        unsafe { ffi::evdi_close(self.raw.as_ptr()) };
        debug!("EvdiHandle dropped");
    }
}

/// A pixel buffer registered with an `EvdiHandle`. Must outlive the
/// registration; use `EvdiHandle::unregister_buffer` before dropping.
pub struct EvdiBuffer {
    pub id: i32,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pixels: Vec<u8>,
    rects: Vec<ffi::evdi_rect>,
    registered_by: Option<usize>,
}

impl EvdiBuffer {
    /// Allocate an XRGB8888 buffer matching the given dimensions.
    pub fn new(id: i32, width: i32, height: i32) -> Self {
        let stride = width * 4;
        let pixels = vec![0u8; (stride * height) as usize];
        let rects = vec![ffi::evdi_rect::default(); MAX_DIRTY_RECTS];
        Self {
            id,
            width,
            height,
            stride,
            pixels,
            rects,
            registered_by: None,
        }
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }
}

impl Drop for EvdiBuffer {
    fn drop(&mut self) {
        if self.registered_by.is_some() {
            // Registration was never cleaned up — the handle is gone but we
            // still have memory evdi may touch. Zero the buffer so stale
            // pixels don't leak on reuse.
            self.pixels.fill(0);
        }
    }
}

/// Tiny smoke test: load libevdi and report its version. Used by the daemon
/// at startup to fail fast when the runtime library is missing.
pub fn probe_runtime() -> Result<(i32, i32, i32)> {
    let v = EvdiHandle::lib_version();
    if v == (0, 0, 0) {
        return Err(anyhow!("libevdi reported version 0.0.0"));
    }
    Ok(v)
}

fn explain_add_failure() -> anyhow::Error {
    const ADD_SYSFS: &str = "/sys/devices/evdi/add";
    let module_present = std::path::Path::new(ADD_SYSFS).exists();
    if !module_present {
        return anyhow!(
            "evdi_add_device failed and {ADD_SYSFS} does not exist — is the evdi \
             kernel module loaded? Try: sudo modprobe evdi"
        );
    }
    let writable = std::fs::OpenOptions::new()
        .write(true)
        .open(ADD_SYSFS)
        .is_ok();
    if writable {
        return anyhow!(
            "evdi_add_device failed despite {ADD_SYSFS} being writable — \
             check dmesg for evdi errors"
        );
    }
    anyhow!(
        "evdi_add_device failed: writing to {ADD_SYSFS} is root-only. Run the \
         daemon as root (systemd does this by default), or pre-create an evdi \
         device once with `sudo sh -c 'echo 1 > {ADD_SYSFS}'` and then the \
         daemon can open it unprivileged"
    )
}

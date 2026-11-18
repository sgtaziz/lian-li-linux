use crate::frame::{Frame, Mode, PixelFormat};
use crate::hyprland::{Control, OwnedOutput};
use crate::{Capture, Event, OutputRequest, MAX_FRAME_BYTES};
use anyhow::{bail, ensure, Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1 as configuration_head,
    zwlr_output_configuration_v1 as configuration, zwlr_output_head_v1 as head,
    zwlr_output_manager_v1 as manager, zwlr_output_mode_v1 as output_mode,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1 as copy_frame, zwlr_screencopy_manager_v1 as copy_manager,
};

const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OUTPUTS: usize = 64;

pub struct HyprlandCapture {
    state: State,
    queue: EventQueue<State>,
    connection: Connection,
    output: OwnedOutput,
}

impl HyprlandCapture {
    pub fn open(control: Control, request: OutputRequest, cancel: &AtomicBool) -> Result<Self> {
        request.validate_mode(request.preferred)?;
        let connection = Connection::from_socket(control.wayland_stream()?)?;
        let mut queue = connection.new_event_queue::<State>();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = State::new(request.preferred);
        let deadline = Instant::now() + SETUP_TIMEOUT;
        while state.manager.is_none()
            || state.copy_manager.is_none()
            || state.shm.is_none()
            || state.serial.is_none()
        {
            ensure!(!cancel.load(Ordering::Relaxed), "Desktop setup cancelled");
            dispatch(&connection, &mut queue, &mut state, deadline)?;
            state.check_error()?;
        }
        ensure!(!cancel.load(Ordering::Relaxed), "Desktop setup cancelled");
        let output = control.create_output(&request.edid)?;
        state.serial = None;
        state.target_name = output.name().to_owned();
        let mut capture = Self {
            state,
            queue,
            connection,
            output,
        };
        let deadline = Instant::now() + SETUP_TIMEOUT;
        while capture.state.serial.is_none()
            || !capture
                .state
                .heads
                .values()
                .any(|head| head.name == capture.state.target_name)
        {
            ensure!(!cancel.load(Ordering::Relaxed), "Desktop setup cancelled");
            capture.dispatch(deadline)?;
        }
        capture.configure()?;
        let deadline = Instant::now() + SETUP_TIMEOUT;
        while !capture.state.configured || capture.state.target_output().is_none() {
            ensure!(!cancel.load(Ordering::Relaxed), "Desktop setup cancelled");
            capture.dispatch(deadline)?;
        }
        let monitor = capture.output.monitor()?;
        let mode = capture.state.mode;
        ensure!(
            monitor.width == mode.width
                && monitor.height == mode.height
                && (monitor.refresh_hz - f64::from(mode.refresh_hz)).abs() < 0.5,
            "Hyprland did not accept the physical panel's requested mode"
        );
        Ok(capture)
    }

    fn dispatch(&mut self, deadline: Instant) -> Result<()> {
        dispatch(&self.connection, &mut self.queue, &mut self.state, deadline)?;
        self.state.check_error()
    }

    fn configure(&mut self) -> Result<()> {
        let state = &self.state;
        let qh = self.queue.handle();
        let manager = state
            .manager
            .as_ref()
            .context("Output management unavailable")?;
        let config = manager.create_configuration(
            state.serial.context("Output configuration is incomplete")?,
            &qh,
            (),
        );
        let head = state
            .heads
            .values()
            .find(|head| head.name == state.target_name)
            .context("Owned output-management head disappeared")?;
        // Hyprland accepts partial configurations but ignores serial validation.
        // Sending only our head preserves concurrent changes to unrelated monitors.
        let target = config.enable_head(&head.proxy, &qh, ());
        target.set_custom_mode(
            state.mode.width as i32,
            state.mode.height as i32,
            state.mode.refresh_hz as i32 * 1000,
        );
        // A Wayland position override would take precedence over later Hyprland rules.
        target.set_scale(1.0);
        target.set_transform(wl_output::Transform::Normal);
        config.apply();
        self.state.configuration = Some(config);
        Ok(())
    }
}

impl Capture for HyprlandCapture {
    fn invalidate(&mut self) -> Result<()> {
        if let Some(frame) = self.state.pending.take() {
            frame.destroy();
        }
        self.state.delivered = false;
        self.state.ready = false;
        self.state.announced = false;
        self.state.buffer.take();
        self.connection.flush()?;
        Ok(())
    }

    fn poll_events(&mut self, timeout: Duration, cancel: &AtomicBool) -> Result<Vec<Event>> {
        ensure!(!cancel.load(Ordering::Relaxed), "Wayland capture cancelled");
        if !self.state.announced {
            self.state.announced = true;
            self.state.first_frame_deadline = Some(Instant::now() + SETUP_TIMEOUT);
            self.request_update()?;
        }
        ensure!(
            self.state.delivered
                || self
                    .state
                    .first_frame_deadline
                    .is_none_or(|deadline| Instant::now() < deadline),
            "Compositor did not deliver the first desktop frame"
        );
        let deadline = Instant::now() + timeout;
        match self.dispatch(deadline) {
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::TimedOut) => {}
            result => result?,
        }
        if self.state.ready {
            self.state.ready = false;
            let buffer = self
                .state
                .buffer
                .as_mut()
                .context("Capture completed without a buffer")?;
            buffer.file.read_exact_at(&mut buffer.pixels, 0)?;
            if self.state.y_invert {
                flip_rows(
                    &mut buffer.pixels,
                    buffer.stride,
                    self.state.mode.height as usize,
                );
            }
            self.state.timestamp = self.state.pending_timestamp.take();
            self.state.damage = if !self.state.delivered || self.state.y_invert {
                crate::frame::FrameDamage::Full
            } else {
                self.state.pending_damage
            };
            let mut events = Vec::with_capacity(2);
            if !self.state.delivered {
                self.state.delivered = true;
                events.push(Event::ModeChanged(self.state.mode, buffer.format));
            }
            events.push(Event::FrameReady);
            return Ok(events);
        }
        Ok(Vec::new())
    }

    fn request_update(&mut self) -> Result<bool> {
        if self.state.pending.is_some() {
            return Ok(false);
        }
        let output = self
            .state
            .target_output()
            .context("Owned Wayland output disappeared")?;
        let manager = self
            .state
            .copy_manager
            .as_ref()
            .context("Capture protocol unavailable")?;
        let frame = manager.capture_output(1, output, &self.queue.handle(), ());
        self.state.pending = Some(frame);
        self.state.pending_timestamp = None;
        self.state.pending_damage = crate::frame::FrameDamage::Unchanged;
        self.state.y_invert = false;
        self.state.buffer_spec = None;
        self.connection.flush()?;
        Ok(false)
    }

    fn frame(&mut self, _: &AtomicBool) -> Result<Frame<'_>> {
        ensure!(
            self.state.delivered,
            "No complete desktop frame has been captured"
        );
        let buffer = self
            .state
            .buffer
            .as_ref()
            .context("Capture buffer unavailable")?;
        Frame::new(
            self.state.mode,
            buffer.format,
            buffer.stride,
            &buffer.pixels,
        )
        .and_then(|frame| frame.with_damage(self.state.damage))
        .map(|frame| frame.with_timestamp(self.state.timestamp))
    }
}

impl Drop for HyprlandCapture {
    fn drop(&mut self) {
        if let Some(frame) = self.state.pending.take() {
            frame.destroy();
        }
        self.state.buffer.take();
        if let Err(error) = self.connection.flush() {
            tracing::debug!("Wayland capture connection closed during cleanup: {error}");
        }
    }
}

struct State {
    mode: Mode,
    target_name: String,
    manager: Option<manager::ZwlrOutputManagerV1>,
    copy_manager: Option<copy_manager::ZwlrScreencopyManagerV1>,
    shm: Option<wl_shm::WlShm>,
    serial: Option<u32>,
    heads: HashMap<wayland_client::backend::ObjectId, Head>,
    modes: HashMap<wayland_client::backend::ObjectId, OutputMode>,
    outputs: HashMap<u32, (wl_output::WlOutput, String)>,
    configuration: Option<configuration::ZwlrOutputConfigurationV1>,
    configured: bool,
    announced: bool,
    delivered: bool,
    first_frame_deadline: Option<Instant>,
    pending: Option<copy_frame::ZwlrScreencopyFrameV1>,
    buffer_spec: Option<BufferSpec>,
    buffer: Option<Buffer>,
    ready: bool,
    timestamp: Option<crate::frame::CaptureTimestamp>,
    pending_timestamp: Option<crate::frame::CaptureTimestamp>,
    damage: crate::frame::FrameDamage,
    pending_damage: crate::frame::FrameDamage,
    y_invert: bool,
    error: Option<String>,
}

impl State {
    fn capture_damage(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<()> {
        let right = x
            .checked_add(width)
            .context("Capture damage width overflow")?;
        let bottom = y
            .checked_add(height)
            .context("Capture damage height overflow")?;
        let damage = crate::frame::FrameDamage::rectangle(self.mode, [x, y, right, bottom])?;
        self.pending_damage = self.pending_damage.union(damage);
        Ok(())
    }

    fn capture_ready(&mut self, seconds_hi: u32, seconds_lo: u32, nanos: u32) -> Result<()> {
        let timestamp = crate::frame::CaptureTimestamp::from_parts(
            (u64::from(seconds_hi) << 32) | u64::from(seconds_lo),
            nanos,
            crate::frame::CaptureClock::Compositor,
        )?;
        self.pending_timestamp = Some(timestamp);
        self.ready = true;
        Ok(())
    }

    fn new(mode: Mode) -> Self {
        Self {
            mode,
            target_name: String::new(),
            manager: None,
            copy_manager: None,
            shm: None,
            serial: None,
            heads: HashMap::new(),
            modes: HashMap::new(),
            outputs: HashMap::new(),
            configuration: None,
            configured: false,
            announced: false,
            delivered: false,
            first_frame_deadline: None,
            pending: None,
            buffer_spec: None,
            buffer: None,
            ready: false,
            timestamp: None,
            pending_timestamp: None,
            damage: crate::frame::FrameDamage::Unknown,
            pending_damage: crate::frame::FrameDamage::Unchanged,
            y_invert: false,
            error: None,
        }
    }

    fn check_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            bail!("{error}");
        }
        Ok(())
    }

    fn target_output(&self) -> Option<&wl_output::WlOutput> {
        self.outputs
            .values()
            .find(|(_, name)| *name == self.target_name)
            .map(|(proxy, _)| proxy)
    }

    fn copy(
        &mut self,
        frame: &copy_frame::ZwlrScreencopyFrameV1,
        qh: &QueueHandle<Self>,
    ) -> Result<()> {
        let spec = self
            .buffer_spec
            .context("Compositor does not offer CPU capture buffers")?;
        ensure!(
            spec.width == self.mode.width && spec.height == self.mode.height,
            "Capture geometry differs from the physical panel mode"
        );
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.spec != spec)
        {
            ensure!(!self.delivered, "Capture layout changed during playback");
            self.buffer = Some(Buffer::new(
                spec,
                self.shm
                    .as_ref()
                    .context("Wayland shared memory unavailable")?,
                qh,
            )?);
        }
        let buffer = &self.buffer.as_ref().unwrap().proxy;
        if self.delivered {
            frame.copy_with_damage(buffer);
        } else {
            frame.copy(buffer);
        }
        Ok(())
    }
}

struct Head {
    proxy: head::ZwlrOutputHeadV1,
    name: String,
    enabled: Option<bool>,
    current: Option<output_mode::ZwlrOutputModeV1>,
    position: (i32, i32),
    scale: f64,
    transform: wl_output::Transform,
}

struct OutputMode {
    size: (i32, i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BufferSpec {
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
}

struct Buffer {
    proxy: wl_buffer::WlBuffer,
    file: File,
    pixels: Vec<u8>,
    stride: usize,
    format: PixelFormat,
    spec: BufferSpec,
}

impl Buffer {
    fn new(spec: BufferSpec, shm: &wl_shm::WlShm, qh: &QueueHandle<State>) -> Result<Self> {
        let format = match spec.format {
            wl_shm::Format::Xrgb8888 => PixelFormat::Xrgb8888,
            wl_shm::Format::Argb8888 => PixelFormat::Argb8888,
            wl_shm::Format::Xbgr8888 => PixelFormat::Xbgr8888,
            wl_shm::Format::Abgr8888 => PixelFormat::Abgr8888,
            _ => bail!("Unsupported Wayland capture pixel format"),
        };
        let size = buffer_size(spec.width, spec.height, spec.stride)?;
        // memfd_create returns a new anonymous descriptor owned exclusively by this buffer.
        let fd = unsafe {
            libc::memfd_create(
                c"lianli-capture".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        ensure!(
            fd >= 0,
            "Creating capture memory: {}",
            io::Error::last_os_error()
        );
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(size as u64)?;
        let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
        // The compositor must write pixels, but must not resize the shared storage.
        ensure!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } >= 0,
            "Sealing capture storage failed"
        );
        let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
        let proxy = pool.create_buffer(
            0,
            spec.width as i32,
            spec.height as i32,
            spec.stride as i32,
            spec.format,
            qh,
            (),
        );
        pool.destroy();
        Ok(Self {
            proxy,
            file,
            pixels: vec![0; size],
            stride: spec.stride as usize,
            format,
            spec,
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.proxy.destroy();
    }
}

fn buffer_size(width: u32, height: u32, stride: u32) -> Result<usize> {
    ensure!(width > 0 && height > 0, "Empty Wayland capture buffer");
    ensure!(
        u64::from(stride) >= u64::from(width) * 4,
        "Invalid Wayland capture stride"
    );
    let size = u64::from(stride) * u64::from(height);
    ensure!(
        size <= MAX_FRAME_BYTES as u64,
        "Wayland capture buffer exceeds 64 MiB"
    );
    Ok(size as usize)
}

fn flip_rows(pixels: &mut [u8], stride: usize, height: usize) {
    for row in 0..height / 2 {
        let opposite = height - row - 1;
        let (top, bottom) = pixels.split_at_mut(opposite * stride);
        top[row * stride..(row + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
}

fn dispatch(
    connection: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
) -> Result<()> {
    let count = queue.dispatch_pending(state)?;
    state.check_error()?;
    connection.flush()?;
    if count > 0 {
        return Ok(());
    }
    let Some(guard) = queue.prepare_read() else {
        return Ok(());
    };
    let mut poll = libc::pollfd {
        fd: connection.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::from(io::ErrorKind::TimedOut).into());
    }
    // The read guard and connection retain the descriptor through this bounded wait.
    let result = unsafe { libc::poll(&mut poll, 1, remaining.as_millis().clamp(1, 100) as i32) };
    if result == 0 {
        if Instant::now() >= deadline {
            return Err(io::Error::from(io::ErrorKind::TimedOut).into());
        }
        return Ok(());
    }
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(());
        }
        return Err(error.into());
    }
    guard.read()?;
    queue.dispatch_pending(state)?;
    state.check_error()
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_output_manager_v1" if version >= 4 => {
                    state.manager = Some(registry.bind(name, 4, qh, ()))
                }
                "zwlr_screencopy_manager_v1" if version >= 3 => {
                    state.copy_manager = Some(registry.bind(name, 3, qh, ()))
                }
                "wl_output" if version >= 4 => {
                    if state.outputs.len() >= MAX_OUTPUTS {
                        state.error = Some("Too many Wayland outputs".into());
                        return;
                    }
                    state
                        .outputs
                        .insert(name, (registry.bind(name, 4, qh, name), String::new()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some((output, name)) = state.outputs.remove(&name) {
                    if name == state.target_name {
                        state.error = Some("Owned Wayland output disappeared".into());
                    }
                    output.release();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = &event {
            if let Some((_, stored)) = state.outputs.get_mut(id) {
                *stored = name.clone();
            }
        }
        let owned = state.configured
            && state
                .outputs
                .get(id)
                .is_some_and(|(_, name)| *name == state.target_name);
        if owned {
            match event {
                wl_output::Event::Mode {
                    flags: WEnum::Value(flags),
                    width,
                    height,
                    refresh,
                } if flags.contains(wl_output::Mode::Current) => {
                    if width != state.mode.width as i32
                        || height != state.mode.height as i32
                        || (i64::from(refresh) - i64::from(state.mode.refresh_hz) * 1000).abs()
                            >= 500
                    {
                        state.error = Some("Owned output mode changed during playback".into());
                    }
                }
                wl_output::Event::Geometry { transform, .. }
                    if transform != WEnum::Value(wl_output::Transform::Normal) =>
                {
                    state.error = Some("Owned output rotation changed during playback".into())
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<manager::ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &manager::ZwlrOutputManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Head { head } => {
                if state.heads.len() >= MAX_OUTPUTS {
                    state.error = Some("Too many output-management heads".into());
                    return;
                }
                state.heads.insert(
                    head.id(),
                    Head {
                        proxy: head,
                        name: String::new(),
                        enabled: None,
                        current: None,
                        position: (0, 0),
                        scale: 1.0,
                        transform: wl_output::Transform::Normal,
                    },
                );
            }
            manager::Event::Done { serial } => state.serial = Some(serial),
            manager::Event::Finished => state.error = Some("Output management stopped".into()),
            _ => {}
        }
    }
    event_created_child!(State, manager::ZwlrOutputManagerV1, [0 => (head::ZwlrOutputHeadV1, ())]);
}

impl Dispatch<head::ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &head::ZwlrOutputHeadV1,
        event: head::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(head) = state.heads.get_mut(&proxy.id()) else {
            return;
        };
        match event {
            head::Event::Name { name } => head.name = name,
            head::Event::Enabled { enabled } => head.enabled = Some(enabled != 0),
            head::Event::CurrentMode { mode } => head.current = Some(mode),
            head::Event::Position { x, y } => head.position = (x, y),
            head::Event::Scale { scale } => head.scale = scale,
            head::Event::Transform {
                transform: WEnum::Value(transform),
            } => head.transform = transform,
            head::Event::Mode { mode } => {
                if state.modes.len() >= 4096 {
                    state.error = Some("Too many output modes".into());
                    return;
                }
                state.modes.insert(mode.id(), OutputMode { size: (0, 0) });
            }
            head::Event::Finished => {
                state.heads.remove(&proxy.id());
                proxy.release();
            }
            _ => {}
        }
    }
    event_created_child!(State, head::ZwlrOutputHeadV1, [3 => (output_mode::ZwlrOutputModeV1, ())]);
}

impl Dispatch<output_mode::ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &output_mode::ZwlrOutputModeV1,
        event: output_mode::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            output_mode::Event::Size { width, height } => {
                if let Some(mode) = state.modes.get_mut(&proxy.id()) {
                    mode.size = (width, height);
                }
            }
            output_mode::Event::Finished => {
                state.modes.remove(&proxy.id());
                proxy.release();
            }
            _ => {}
        }
    }
}

impl Dispatch<configuration::ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &configuration::ZwlrOutputConfigurationV1,
        event: configuration::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            configuration::Event::Succeeded => state.configured = true,
            configuration::Event::Failed | configuration::Event::Cancelled => {
                state.error =
                    Some("Compositor rejected or superseded the headless mode configuration".into())
            }
            _ => {}
        }
        proxy.destroy();
        state.configuration = None;
    }
}

impl Dispatch<copy_frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &copy_frame::ZwlrScreencopyFrameV1,
        event: copy_frame::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if state.pending.as_ref() != Some(proxy) {
            return;
        }
        match event {
            copy_frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                state.buffer_spec = Some(BufferSpec {
                    width,
                    height,
                    stride,
                    format,
                })
            }
            copy_frame::Event::BufferDone => {
                if let Err(error) = state.copy(proxy, qh) {
                    state.error = Some(format!("{error:#}"));
                }
            }
            copy_frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => state.y_invert = flags.contains(copy_frame::Flags::YInvert),
            copy_frame::Event::Ready {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                if let Err(error) = state.capture_ready(tv_sec_hi, tv_sec_lo, tv_nsec) {
                    state.error = Some(format!("{error:#}"));
                }
                state.pending = None;
                proxy.destroy();
            }
            copy_frame::Event::Damage {
                x,
                y,
                width,
                height,
            } => {
                if let Err(error) = state.capture_damage(x, y, width, height) {
                    state.error = Some(format!("{error:#}"));
                }
            }
            copy_frame::Event::Failed => {
                state.error = Some("Compositor denied or failed headless capture".into());
                state.pending = None;
                proxy.destroy();
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore configuration_head::ZwlrOutputConfigurationHeadV1);
delegate_noop!(State: ignore copy_manager::ZwlrScreencopyManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_events_accumulate_without_changing_the_published_frame() {
        use crate::frame::FrameDamage;
        let mut state = State::new(Mode {
            width: 1920,
            height: 480,
            refresh_hz: 30,
        });
        state.damage = FrameDamage::Full;
        state.capture_damage(10, 20, 30, 40).unwrap();
        state.capture_damage(1900, 460, 20, 20).unwrap();
        let pending = FrameDamage::Rectangle([10, 20, 1920, 480]);
        assert_eq!(state.pending_damage, pending);
        assert_eq!(state.damage, FrameDamage::Full);
        assert!(state.capture_damage(u32::MAX, 0, 1, 1).is_err());
        assert!(state.capture_damage(0, u32::MAX, 1, 1).is_err());
        assert!(state.capture_damage(1900, 460, 21, 20).is_err());
        assert_eq!(state.pending_damage, pending);
    }

    #[test]
    fn ready_timestamp_waits_for_pixel_publication_and_rejects_invalid_parts() {
        let mut state = State::new(Mode {
            width: 480,
            height: 480,
            refresh_hz: 30,
        });
        let previous = crate::frame::CaptureTimestamp::from_parts(
            1,
            2,
            crate::frame::CaptureClock::Compositor,
        )
        .unwrap();
        state.timestamp = Some(previous);
        assert!(state.capture_ready(1, 2, 1_000_000_000).is_err());
        assert!(!state.ready);
        assert!(state.pending_timestamp.is_none());
        state.capture_ready(1, 2, 3).unwrap();
        assert!(state.ready);
        let pending = state.pending_timestamp.unwrap();
        assert_eq!(pending.since_epoch.as_secs(), (1u64 << 32) + 2);
        assert_eq!(pending.since_epoch.subsec_nanos(), 3);
        assert_eq!(state.timestamp, Some(previous));
    }

    #[test]
    fn split_compositor_event_is_handled_once_the_rest_arrives() {
        use std::io::Write;
        let (client, mut compositor) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let mut queue = connection.new_event_queue::<State>();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut state = State::new(Mode {
            width: 480,
            height: 480,
            refresh_hz: 30,
        });
        let (global_opcode, global_name, version, wl_shm_with_nul) = (0u32, 7u32, 1u32, 7u32);
        let interface = b"wl_shm\0\0";
        let event_bytes = 8 + 4 + 4 + interface.len() as u32 + 4;
        let mut event = Vec::new();
        event.extend_from_slice(&registry.id().protocol_id().to_ne_bytes());
        event.extend_from_slice(&((event_bytes << 16) | global_opcode).to_ne_bytes());
        event.extend_from_slice(&global_name.to_ne_bytes());
        event.extend_from_slice(&wl_shm_with_nul.to_ne_bytes());
        event.extend_from_slice(interface);
        event.extend_from_slice(&version.to_ne_bytes());
        let deadline = Instant::now() + Duration::from_secs(1);

        compositor.write_all(&event[..12]).unwrap();
        dispatch(&connection, &mut queue, &mut state, deadline).unwrap();
        assert!(state.shm.is_none());

        compositor.write_all(&event[12..]).unwrap();
        dispatch(&connection, &mut queue, &mut state, deadline).unwrap();
        assert!(state.shm.is_some());
    }

    #[test]
    fn capture_buffer_limits_include_padding_and_reject_overflow() {
        assert_eq!(buffer_size(480, 480, 2048).unwrap(), 983040);
        assert!(buffer_size(480, 480, 1919).is_err());
        assert!(buffer_size(1, u32::MAX, u32::MAX).is_err());
        assert!(buffer_size(0, 480, 2048).is_err());
    }

    #[test]
    fn inverted_capture_rows_preserve_channels_and_padding() {
        let mut pixels = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0];
        flip_rows(&mut pixels, 4, 3);
        assert_eq!(pixels, [7, 8, 9, 0, 4, 5, 6, 0, 1, 2, 3, 0]);
    }
}

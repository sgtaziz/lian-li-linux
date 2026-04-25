use crate::RusbHidTransport;
use hidapi::HidDevice;
use std::sync::Arc;
use tracing::{info, warn};

/// Closure that produces a fresh inner backend after a stale-handle event
/// (USB suspend/resume, hub reset, transient unplug).
///
/// Wired up at construction so the transport can self-heal without each
/// controller having to plumb its own retry logic.
pub type HidReopener = Arc<dyn Fn() -> anyhow::Result<HidBackendKind> + Send + Sync>;

pub enum HidBackendKind {
    Hidapi(HidDevice),
    Rusb(RusbHidTransport),
    Closed,
}

pub struct HidBackend {
    kind: HidBackendKind,
    reopener: Option<HidReopener>,
}

impl HidBackend {
    pub fn from_hidapi(dev: HidDevice) -> Self {
        Self {
            kind: HidBackendKind::Hidapi(dev),
            reopener: None,
        }
    }

    pub fn from_rusb(transport: RusbHidTransport) -> Self {
        Self {
            kind: HidBackendKind::Rusb(transport),
            reopener: None,
        }
    }

    pub fn with_reopener(mut self, reopener: HidReopener) -> Self {
        self.reopener = Some(reopener);
        self
    }

    pub fn set_reopener(&mut self, reopener: HidReopener) {
        self.reopener = Some(reopener);
    }

    fn try_reopen(&mut self) -> anyhow::Result<()> {
        let reopener = self
            .reopener
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no reopener configured"))?;
        drop(std::mem::replace(&mut self.kind, HidBackendKind::Closed));
        let new_kind = reopener().map_err(|e| anyhow::anyhow!("reopen: {e}"))?;
        self.kind = new_kind;
        Ok(())
    }

    fn do_write(&self, data: &[u8]) -> anyhow::Result<usize> {
        match &self.kind {
            HidBackendKind::Hidapi(dev) => dev.write(data).map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Rusb(dev) => dev.write(data).map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Closed => Err(anyhow::anyhow!("HID backend closed")),
        }
    }

    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<usize> {
        match self.do_write(data) {
            Ok(n) => Ok(n),
            Err(e) if self.reopener.is_some() => {
                warn!("HID write failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("HID handle reopened, retrying write");
                self.do_write(data)
            }
            Err(e) => Err(e),
        }
    }

    fn do_read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> anyhow::Result<usize> {
        match &self.kind {
            HidBackendKind::Hidapi(dev) => dev
                .read_timeout(buf, timeout_ms)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Rusb(dev) => dev
                .read_timeout(buf, timeout_ms)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Closed => Err(anyhow::anyhow!("HID backend closed")),
        }
    }

    pub fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> anyhow::Result<usize> {
        match self.do_read_timeout(buf, timeout_ms) {
            Ok(n) => Ok(n),
            Err(e) if self.reopener.is_some() => {
                warn!("HID read_timeout failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("HID handle reopened, retrying read_timeout");
                self.do_read_timeout(buf, timeout_ms)
            }
            Err(e) => Err(e),
        }
    }

    fn do_send_feature_report(&self, data: &[u8]) -> anyhow::Result<()> {
        match &self.kind {
            HidBackendKind::Hidapi(dev) => dev
                .send_feature_report(data)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Rusb(dev) => {
                dev.send_feature_report(data)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(())
            }
            HidBackendKind::Closed => Err(anyhow::anyhow!("HID backend closed")),
        }
    }

    pub fn send_feature_report(&mut self, data: &[u8]) -> anyhow::Result<()> {
        match self.do_send_feature_report(data) {
            Ok(()) => Ok(()),
            Err(e) if self.reopener.is_some() => {
                warn!("HID send_feature_report failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("HID handle reopened, retrying send_feature_report");
                self.do_send_feature_report(data)
            }
            Err(e) => Err(e),
        }
    }

    fn do_get_feature_report(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match &self.kind {
            HidBackendKind::Hidapi(dev) => dev
                .get_feature_report(buf)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Rusb(dev) => dev
                .get_feature_report(buf)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Closed => Err(anyhow::anyhow!("HID backend closed")),
        }
    }

    pub fn get_feature_report(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self.do_get_feature_report(buf) {
            Ok(n) => Ok(n),
            Err(e) if self.reopener.is_some() => {
                warn!("HID get_feature_report failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("HID handle reopened, retrying get_feature_report");
                self.do_get_feature_report(buf)
            }
            Err(e) => Err(e),
        }
    }

    fn do_get_input_report(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match &self.kind {
            HidBackendKind::Hidapi(dev) => dev
                .get_input_report(buf)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Rusb(dev) => dev
                .get_input_report(buf)
                .map_err(|e| anyhow::anyhow!("{e}")),
            HidBackendKind::Closed => Err(anyhow::anyhow!("HID backend closed")),
        }
    }

    pub fn get_input_report(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self.do_get_input_report(buf) {
            Ok(n) => Ok(n),
            Err(e) if self.reopener.is_some() => {
                warn!("HID get_input_report failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("HID handle reopened, retrying get_input_report");
                self.do_get_input_report(buf)
            }
            Err(e) => Err(e),
        }
    }

    /// Drain any stale data from the device read buffer.
    pub fn read_flush(&mut self) {
        let mut buf = [0u8; 64];
        loop {
            match self.do_read_timeout(&mut buf, 5) {
                Ok(n) if n > 0 => continue,
                _ => break,
            }
        }
    }
}

use thiserror::Error;

/// Errors raised by the transport layer.
///
/// All HID I/O now goes via `rusb` (libusb); the legacy `Hid` variant from the
/// previous `hidapi` backend is gone.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),

    #[error("device {vid:04x}:{pid:04x} not found")]
    DeviceNotFound { vid: u16, pid: u16 },

    #[error("write failed: {0}")]
    Write(String),

    #[error("read failed: {0}")]
    Read(String),

    #[error("timeout")]
    Timeout,

    #[error("{0}")]
    Other(String),
}

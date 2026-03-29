use crate::RusbHidTransport;
use hidapi::HidDevice;

pub enum HidBackend {
    Hidapi(HidDevice),
    Rusb(RusbHidTransport),
}

impl HidBackend {
    pub fn write(&self, data: &[u8]) -> anyhow::Result<usize> {
        match self {
            Self::Hidapi(dev) => dev.write(data).map_err(|e| anyhow::anyhow!("{e}")),
            Self::Rusb(dev) => dev.write(data).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }

    pub fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> anyhow::Result<usize> {
        match self {
            Self::Hidapi(dev) => dev.read_timeout(buf, timeout_ms).map_err(|e| anyhow::anyhow!("{e}")),
            Self::Rusb(dev) => dev.read_timeout(buf, timeout_ms).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }

    pub fn send_feature_report(&self, data: &[u8]) -> anyhow::Result<()> {
        match self {
            Self::Hidapi(dev) => {
                dev.send_feature_report(data).map_err(|e| anyhow::anyhow!("{e}"))
            }
            Self::Rusb(dev) => {
                dev.send_feature_report(data).map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(())
            }
        }
    }

    pub fn get_feature_report(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self {
            Self::Hidapi(dev) => dev.get_feature_report(buf).map_err(|e| anyhow::anyhow!("{e}")),
            Self::Rusb(dev) => dev.get_feature_report(buf).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }

    pub fn get_input_report(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self {
            Self::Hidapi(dev) => dev.get_input_report(buf).map_err(|e| anyhow::anyhow!("{e}")),
            Self::Rusb(dev) => dev.get_input_report(buf).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }
}

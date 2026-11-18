pub mod acquire;
pub mod capture;
pub mod uapi;

use crate::OutputRequest;
use anyhow::{ensure, Context, Result};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub struct Node {
    file: File,
    pub caps: uapi::Caps,
    pub identity: uapi::Identity,
}

impl Node {
    pub fn candidates() -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir("/dev/dri")?.take(1024) {
            let entry = entry?;
            if is_render_node(&entry.file_name().to_string_lossy()) {
                ensure!(paths.len() < 128, "Too many DRM render nodes");
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
    }

    pub fn open(path: &Path) -> Result<Self> {
        ensure!(
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_render_node),
            "Hermes capture requires a render node"
        );
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        ensure!(
            file.metadata()?.file_type().is_char_device(),
            "DRM candidate is not a device node"
        );
        verify_driver(&file)?;
        let mut version = uapi::Version::default();
        call(&file, &mut version)?;
        ensure!(
            version.uapi == uapi::UAPI_VERSION && name(&version.driver_name)? == uapi::DRIVER_NAME,
            "Unsupported Hermes-KMS UAPI {}. Expected {}.",
            version.uapi,
            uapi::UAPI_VERSION
        );
        let mut caps = uapi::Caps::default();
        call(&file, &mut caps)?;
        ensure!(
            caps.flags & uapi::REQUIRED_CAPS == uapi::REQUIRED_CAPS,
            "Hermes-KMS is missing required capture/cursor/synchronization capabilities"
        );
        ensure!(
            (1..=8).contains(&caps.output_count),
            "Unsupported Hermes-KMS output count"
        );
        let mut identity = uapi::Identity::default();
        call(&file, &mut identity)?;
        validate_identity(&identity, caps.output_count)?;
        Ok(Self {
            file,
            caps,
            identity,
        })
    }

    pub fn select(&mut self, index: u32) -> Result<()> {
        ensure!(
            index < self.caps.output_count,
            "Hermes output index is out of range"
        );
        let mut select = uapi::SelectOutput {
            output_index: index,
            ..Default::default()
        };
        call(&self.file, &mut select)?;
        ensure!(
            select.selected_output_index == index && select.output_count == self.caps.output_count,
            "Hermes output selection changed unexpectedly"
        );
        call(&self.file, &mut self.identity)?;
        validate_identity(&self.identity, self.caps.output_count)?;
        ensure!(
            self.identity.output_index == index,
            "Hermes returned another output identity"
        );
        Ok(())
    }

    pub fn claim(self, request: &OutputRequest) -> Result<OwnedOutput> {
        validate_mode(&self.caps, request)?;
        let mut status = uapi::Status::default();
        call(&self.file, &mut status)?;
        ensure!(
            status.flags & ((1 << 0) | (1 << 2) | (1 << 3) | (1 << 5)) == 0
                && status.session_id == 0,
            "Hermes output is already enabled, owned or in use"
        );
        ensure!(
            status.flags & (1 << 6) != 0,
            "Hermes hotplug events are disabled. The compositor cannot activate this output."
        );
        let preferred = request.preferred;
        let mut enable = uapi::SetOutput {
            enabled: 1,
            width: preferred.width,
            height: preferred.height,
            refresh_hz: preferred.refresh_hz,
            ..Default::default()
        };
        call(&self.file, &mut enable)?;
        // The fd owns any successful claim even if response validation below fails.
        let output = OwnedOutput {
            node: self,
            session_id: enable.session_id,
        };
        ensure!(
            enable.session_id != 0 && enable.result_flags & 3 == 3,
            "Hermes did not confirm exclusive output ownership"
        );
        ensure!(
            (enable.width, enable.height, enable.refresh_hz)
                == (preferred.width, preferred.height, preferred.refresh_hz),
            "Hermes changed the requested panel mode"
        );
        Ok(output)
    }
}

pub struct OwnedOutput {
    node: Node,
    session_id: u64,
}

impl OwnedOutput {
    pub fn identity(&self) -> &uapi::Identity {
        &self.node.identity
    }

    pub fn status(&self) -> Result<uapi::Status> {
        let mut status = uapi::Status::default();
        call(&self.node.file, &mut status)?;
        ensure!(
            status.session_id == self.session_id && status.flags & (1 << 5) != 0,
            "Hermes output ownership was lost"
        );
        Ok(status)
    }
}

impl Drop for OwnedOutput {
    fn drop(&mut self) {
        // Disabling is owner-checked by the kernel; closing the fd also revokes the session.
        if let Err(error) = call(&self.node.file, &mut uapi::SetOutput::default()) {
            tracing::debug!(
                "Hermes output disable failed; releasing its owner descriptor: {error}"
            );
        }
    }
}

fn validate_identity(identity: &uapi::Identity, output_count: u32) -> Result<()> {
    ensure!(
        name(&identity.driver_name)? == uapi::DRIVER_NAME,
        "Hermes driver identity changed"
    );
    ensure!(
        matches!(identity.device_role, 0 | 1) && identity.session_index == 0,
        "Hermes private-session devices are reserved for their own compositors"
    );
    ensure!(
        identity.output_count == output_count && identity.output_index < output_count,
        "Hermes returned inconsistent output identity"
    );
    ensure!(
        identity.connector_id != 0 && identity.cursor_plane_id != 0,
        "Hermes output/cursor objects are unavailable"
    );
    Ok(())
}

fn validate_mode(caps: &uapi::Caps, request: &OutputRequest) -> Result<()> {
    request.validate()?;
    let mode = request.preferred;
    ensure!(
        (caps.min_width..=caps.max_width).contains(&mode.width)
            && (caps.min_height..=caps.max_height).contains(&mode.height)
            && mode.refresh_hz <= caps.max_refresh_hz,
        "Panel mode {}×{}@{} exceeds Hermes-KMS limits {}..={}×{}..={} up to {} Hz",
        mode.width,
        mode.height,
        mode.refresh_hz,
        caps.min_width,
        caps.max_width,
        caps.min_height,
        caps.max_height,
        caps.max_refresh_hz
    );
    Ok(())
}

fn is_render_node(name: &str) -> bool {
    name.strip_prefix("renderD").is_some_and(|index| {
        !index.is_empty() && index.len() <= 10 && index.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn name(value: &[u8]) -> Result<&[u8]> {
    let length = value
        .iter()
        .position(|byte| *byte == 0)
        .context("Unterminated DRM identity")?;
    Ok(&value[..length])
}

fn call<T: uapi::Request>(file: &File, request: &mut T) -> Result<()> {
    // Only sealed, ABI-checked request types can reach this Hermes-verified descriptor.
    if unsafe { libc::ioctl(file.as_raw_fd(), T::IOCTL, request as *mut T) } < 0 {
        return Err(io::Error::last_os_error()).context("Hermes-KMS request failed");
    }
    Ok(())
}

#[repr(C)]
struct DrmVersion {
    major: i32,
    minor: i32,
    patch: i32,
    name_len: usize,
    name: *mut u8,
    date_len: usize,
    date: *mut u8,
    description_len: usize,
    description: *mut u8,
}

fn verify_driver(file: &File) -> Result<()> {
    let mut name = [0u8; 32];
    let mut version = DrmVersion {
        major: 0,
        minor: 0,
        patch: 0,
        name_len: name.len(),
        name: name.as_mut_ptr(),
        date_len: 0,
        date: std::ptr::null_mut(),
        description_len: 0,
        description: std::ptr::null_mut(),
    };
    let request =
        (3u32 << 30) | ((std::mem::size_of::<DrmVersion>() as u32) << 16) | (u32::from(b'd') << 8);
    // DRM_IOCTL_VERSION is a core query, safe before any driver-private command.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            request as libc::c_ulong,
            &mut version as *mut DrmVersion,
        )
    };
    ensure!(result == 0, "DRM core driver identity query failed");
    ensure!(
        version.name_len <= name.len() && &name[..version.name_len] == uapi::DRIVER_NAME,
        "DRM node is not Hermes-KMS"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Mode;

    #[test]
    fn rejects_private_outputs_and_modes_smaller_than_module_limits() {
        let mut identity = uapi::Identity {
            output_count: 1,
            connector_id: 42,
            cursor_plane_id: 43,
            ..Default::default()
        };
        identity.driver_name[..uapi::DRIVER_NAME.len()].copy_from_slice(uapi::DRIVER_NAME);
        validate_identity(&identity, 1).unwrap();
        identity.device_role = 2;
        assert!(validate_identity(&identity, 1).is_err());
        let mode = Mode {
            width: 480,
            height: 480,
            refresh_hz: 30,
        };
        let output = OutputRequest {
            edid: vec![0; 128],
            preferred: mode,
            modes: vec![mode],
            max_width: 480,
            max_height: 480,
            max_frame_bytes: None,
        };
        let mut caps = uapi::Caps {
            min_width: 640,
            min_height: 480,
            max_width: 4096,
            max_height: 4096,
            max_refresh_hz: 144,
            ..Default::default()
        };
        assert!(validate_mode(&caps, &output).is_err());
        caps.min_width = 320;
        validate_mode(&caps, &output).unwrap();
        assert!(!is_render_node("card0"));
        assert!(!is_render_node("renderD128/../../card0"));
        assert!(is_render_node("renderD129"));
    }

    #[test]
    fn panel_modes_must_fit_both_vendor_geometry_and_module_limits() {
        let caps = uapi::Caps {
            min_width: 320,
            min_height: 320,
            max_width: 1920,
            max_height: 1920,
            max_refresh_hz: 60,
            ..Default::default()
        };
        for (width, height, refresh_hz) in [
            (320, 320, 15),
            (480, 480, 30),
            (480, 1920, 30),
            (1920, 480, 60),
        ] {
            let mode = Mode {
                width,
                height,
                refresh_hz,
            };
            let mut request = OutputRequest {
                edid: vec![0; 128],
                preferred: mode,
                modes: vec![mode],
                max_width: width,
                max_height: height,
                max_frame_bytes: None,
            };
            validate_mode(&caps, &request).unwrap();
            assert!(validate_mode(
                &uapi::Caps {
                    max_width: width - 1,
                    ..caps
                },
                &request
            )
            .is_err());
            assert!(validate_mode(
                &uapi::Caps {
                    max_height: height - 1,
                    ..caps
                },
                &request
            )
            .is_err());
            assert!(validate_mode(
                &uapi::Caps {
                    max_refresh_hz: refresh_hz - 1,
                    ..caps
                },
                &request
            )
            .is_err());
            request.preferred.refresh_hz = refresh_hz - 1;
            assert!(validate_mode(&caps, &request).is_err());
            request.preferred = Mode {
                width: height,
                height: width,
                refresh_hz,
            };
            if width != height {
                assert!(validate_mode(&caps, &request).is_err());
            }
        }
    }
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const IPC_PROTOCOL_VERSION: u32 = 1;
pub const GUARDED_WRITES: &str = "guarded_writes";
pub const GRACEFUL_SHUTDOWN: &str = "graceful_shutdown";
pub const SERVICE_STOP: &str = "service_stop";
pub const SERVICE_WRITE_GATE: &str = "service_write_gate";
pub const SERVICE_SELECTION: &str = "service_selection";
pub const SERVICE_STARTUP_GATE: &str = "service_startup_gate";
pub const MEDIA_DECODE: &str = "media_decode";
pub const INSTALLATION_HEALTH: &str = "installation_health";
pub const DESKTOP_RETRY: &str = "desktop_retry";
pub const SENSOR_PREVIEW: &str = "sensor_preview";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonBuildInfo {
    pub version: String,
    pub protocol_version: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

pub fn parse_service_invocation(value: &str) -> Result<String, String> {
    if value.len() != 32
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err("Service invocation must be a nonzero 32-character hexadecimal ID".into());
    }
    Ok(value.to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteGuard {
    pub client_version: String,
    pub protocol_version: u32,
    pub instance_id: String,
}

impl DaemonInfo {
    pub fn write_guard(&self, client_version: &str) -> Result<WriteGuard, String> {
        if self.version != client_version || self.protocol_version != IPC_PROTOCOL_VERSION {
            return Err(format!(
                "Changes are disabled: GUI/client {client_version} and daemon {} must use matching versions. Update both and restart the selected daemon cleanly.",
                self.version
            ));
        }
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == GUARDED_WRITES)
        {
            return Err("Changes are disabled: this daemon lacks compatibility checks. Update both applications and restart the selected daemon cleanly.".into());
        }
        Ok(WriteGuard {
            client_version: client_version.into(),
            protocol_version: IPC_PROTOCOL_VERSION,
            instance_id: self.instance_id.clone(),
        })
    }
}

impl WriteGuard {
    pub fn validate(&self, daemon: &DaemonInfo) -> Result<(), String> {
        let expected = daemon.write_guard(&self.client_version)?;
        if self != &expected {
            return Err("The daemon changed or the IPC version is incompatible. Refresh before applying changes.".into());
        }
        Ok(())
    }
}

/// Configuration scope selected at launch; this does not imply systemd ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    User,
    System,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub version: String,
    pub protocol_version: u32,
    pub instance_id: String,
    pub pid: u32,
    pub mode: DaemonMode,
    pub config_path: PathBuf,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub ownership_lock: Option<FileIdentity>,
    #[serde(default)]
    pub service_invocation: Option<String>,
    #[serde(default)]
    pub service_operation_lock: Option<FileIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: String,
    pub inode: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::IpcRequest;
    use serde_json::json;

    #[test]
    fn installation_health_is_a_read_only_request() {
        let request: crate::ipc::IpcRequest =
            serde_json::from_str(r#"{"method":"GetInstallationHealth"}"#).unwrap();
        assert!(request.is_read_only());
        assert!(request.authorize(&current()).is_ok());
    }

    fn current() -> DaemonInfo {
        DaemonInfo {
            version: "1.0.0".into(),
            protocol_version: IPC_PROTOCOL_VERSION,
            instance_id: "instance-a".into(),
            pid: 123,
            mode: DaemonMode::User,
            config_path: "/example/config.json".into(),
            capabilities: vec![GUARDED_WRITES.into()],
            ownership_lock: None,
            service_invocation: None,
            service_operation_lock: None,
        }
    }

    #[test]
    fn service_invocation_validation_and_legacy_identity_defaults() {
        let valid = "abcdef0123456789abcdef0123456789";
        assert_eq!(
            parse_service_invocation(&valid.to_ascii_uppercase()).unwrap(),
            valid
        );
        for invalid in [
            "",
            "0",
            "00000000000000000000000000000000",
            "g123456789abcdef0123456789abcdef",
            " abcdef0123456789abcdef0123456789",
        ] {
            assert!(parse_service_invocation(invalid).is_err());
        }
        let mut legacy = serde_json::to_value(current()).unwrap();
        legacy.as_object_mut().unwrap().remove("service_invocation");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("service_operation_lock");
        let legacy = serde_json::from_value::<DaemonInfo>(legacy).unwrap();
        assert!(legacy.service_invocation.is_none());
        assert!(legacy.service_operation_lock.is_none());
        let request = IpcRequest::StopService {
            invocation_id: valid.into(),
        };
        assert!(!request.is_read_only());
        assert!(request.authorize(&current()).is_err());
    }

    #[test]
    fn writes_require_matching_versions_capabilities_and_instance() {
        let daemon = current();
        let guard = daemon.write_guard("1.0.0").unwrap();
        guard.validate(&daemon).unwrap();
        assert!(daemon.write_guard("0.9.1").is_err());
        let mut changed = daemon.clone();
        changed.instance_id = "instance-b".into();
        assert!(guard.validate(&changed).is_err());
        changed = daemon.clone();
        changed.protocol_version += 1;
        assert!(guard.validate(&changed).is_err());
        changed = daemon.clone();
        changed.capabilities.clear();
        assert!(guard.validate(&changed).is_err());
    }

    #[test]
    fn legacy_reads_survive_but_writes_and_nested_guards_are_rejected() {
        let daemon = current();
        assert!(IpcRequest::GetConfig.authorize(&daemon).is_ok());
        let write = IpcRequest::SetLcdTemplates { templates: vec![] };
        assert!(write.clone().authorize(&daemon).is_err());
        let guarded = IpcRequest::Guarded {
            guard: daemon.write_guard("1.0.0").unwrap(),
            request: Box::new(write),
        };
        let wire = serde_json::to_value(&guarded).unwrap();
        let decoded: IpcRequest = serde_json::from_value(wire).unwrap();
        assert!(matches!(
            decoded.authorize(&daemon).unwrap(),
            IpcRequest::SetLcdTemplates { .. }
        ));
        let nested = IpcRequest::Guarded {
            guard: daemon.write_guard("1.0.0").unwrap(),
            request: Box::new(guarded),
        };
        assert!(nested.authorize(&daemon).is_err());
    }

    #[test]
    fn older_identity_payloads_can_omit_capabilities() {
        let info: DaemonInfo = serde_json::from_value(json!({
            "version": "1.0.0", "protocol_version": 1, "instance_id": "a",
            "pid": 123, "mode": "system", "config_path": "/var/lib/lianli/config.json"
        }))
        .unwrap();
        assert_eq!(info.mode, DaemonMode::System);
        assert!(info.capabilities.is_empty());
    }

    #[test]
    fn future_modes_and_capabilities_do_not_break_identity_parsing() {
        let info: DaemonInfo = serde_json::from_value(json!({
            "version": "2.0.0", "protocol_version": 2, "instance_id": "b",
            "pid": 456, "mode": "future", "config_path": "/example/config.json",
            "capabilities": ["future_feature"], "future_field": true
        }))
        .unwrap();
        assert_eq!(info.mode, DaemonMode::Unknown);
        assert_eq!(info.capabilities, ["future_feature"]);
    }

    #[test]
    fn identity_request_preserves_existing_ping_wire_format() {
        assert_eq!(
            serde_json::to_value(IpcRequest::Ping).unwrap(),
            json!({"method": "Ping"})
        );
        let request: IpcRequest = serde_json::from_value(json!({
            "method": "GetDaemonInfo", "params": null
        }))
        .unwrap();
        assert!(matches!(request, IpcRequest::GetDaemonInfo));
    }
}

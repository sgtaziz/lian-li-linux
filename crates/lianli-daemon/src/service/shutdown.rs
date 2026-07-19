use super::ServiceManager;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

impl ServiceManager {
    pub(super) fn shutdown(&mut self) {
        self.desktop_displays.shutdown();

        for target in self.targets.values_mut() {
            target.stop();
        }
        self.targets.clear();

        // Controllers (fan / AIO / RGB / direct-color writer)
        self.controllers.shutdown();

        // Drop RGB controller reference from IPC state before clearing HID
        // backends so device handles are released cleanly.
        self.ipc.state.lock().rgb_controller = None;
        self.wired_fan_devices = Arc::new(HashMap::new());
        self.hid_backends.clear();

        self.wireless.stop();
        self.openrgb.shutdown();
        self.ipc.shutdown();
        let _ = info!("Daemon shutdown complete.");
    }
}

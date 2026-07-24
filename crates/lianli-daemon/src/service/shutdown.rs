use super::ServiceManager;
use tracing::info;

impl ServiceManager {
    pub(super) fn shutdown(&mut self) {
        self.desktop_displays.shutdown();

        let mut targets = self.targets.lock();
        for target in targets.values_mut() {
            target.stop();
        }
        targets.clear();

        // Controllers (fan / AIO / RGB / direct-color writer)
        self.controllers.shutdown();

        // Drop RGB controller reference from IPC state before clearing the
        // device registry so device handles are released cleanly.
        self.ipc.state.lock().rgb_controller = None;
        self.registry.clear();

        self.wireless.stop();
        self.openrgb.shutdown();
        self.ipc.shutdown();
        let _ = info!("Daemon shutdown complete.");
    }
}

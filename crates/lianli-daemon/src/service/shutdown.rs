use super::ServiceManager;
use tracing::info;

impl ServiceManager {
    pub(super) fn shutdown(&mut self) {
        let t0 = std::time::Instant::now();
        let mark = |step: &str, t0: std::time::Instant| {
            info!("shutdown step '{step}' done at {:?}", t0.elapsed());
        };

        info!("shutdown: begin");
        lianli_transport::usb::SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);

        // The direct-color writer shares this stop flag and is joined below.
        self.openrgb
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);

        if let Some(writer) = self.controllers.direct_color_writer.take() {
            if writer.join().is_err() {
                tracing::warn!("Direct RGB writer panicked during shutdown");
            }
        }

        if let Some(rgb) = &self.controllers.rgb {
            rgb.lock().stop();
        }

        self.cancel_pixel_clean_preparation();
        self.pixel_clean_sessions.clear();
        self.desktop_displays.shutdown();
        mark("desktop_displays", t0);

        let mut targets = std::mem::take(&mut *self.targets.lock());
        for target in targets.values_mut() {
            if let Err(e) = target.shutdown(&mut self.packet_builder) {
                tracing::warn!(
                    "Failed to turn off LCD {} during shutdown: {e:#}",
                    target.device_identity
                );
            }
        }
        drop(targets);
        mark("targets", t0);

        // Controllers (fan / AIO / RGB / direct-color writer)
        self.controllers.shutdown();
        mark("controllers", t0);

        // Drop RGB controller reference from IPC state before clearing the
        // device registry so device handles are released cleanly.
        {
            let mut state = self.ipc.state.lock();
            state.rgb_controller = None;
            state.pixel_clean_states.clear();
        }
        self.registry.clear();
        mark("registry", t0);

        self.wireless.stop();
        mark("wireless", t0);
        self.openrgb.shutdown();
        mark("openrgb", t0);
        self.ipc.shutdown();
        mark("ipc", t0);
        info!("Daemon shutdown complete in {:?}", t0.elapsed());
    }
}

use super::ServiceManager;
use tracing::info;

impl ServiceManager {
    pub(super) fn shutdown(&mut self) {
        // Each step joins worker threads. If any of them is parked inside a
        // blocking USB call, join() never returns and the signal handler
        // eventually forces the process down mid-transfer, which hangs the
        // device MCU. These markers say exactly which step stalled.
        let t0 = std::time::Instant::now();
        let mark = |step: &str, t0: std::time::Instant| {
            info!("shutdown step '{step}' done at {:?}", t0.elapsed());
        };

        info!("shutdown: begin");

        // FIX: the direct-color writer thread is spawned with a clone of the
        // OpenRGB stop flag (init.rs), but Controllers::shutdown() joins that
        // thread while the flag is still false — openrgb.shutdown() only raises
        // it further down, after the join it is waiting on. The writer loops
        // forever, join() never returns, and the signal handler eventually
        // forces the process down with a USB transfer in flight, which is what
        // leaves the HydroShift II MCU unresponsive. Raise it up front.
        self.openrgb
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);

        self.desktop_displays.shutdown();
        mark("desktop_displays", t0);

        let mut targets = self.targets.lock();
        // Temporarily permit shutdown teardown packet through transport
        lianli_transport::usb::SHUTTING_DOWN
            .store(false, std::sync::atomic::Ordering::Relaxed);
        for target in targets.values_mut() {
            if let Err(e) = target.lcd.set_brightness(
                Some(&self.wireless),
                &mut self.packet_builder,
                0,
            ) {
                tracing::warn!(
                    "Failed to turn off LCD brightness on shutdown for {}: {e}",
                    target.device_identity
                );
            }
            target.stop();
        }
        targets.clear();
        drop(targets);
        lianli_transport::usb::SHUTTING_DOWN
            .store(true, std::sync::atomic::Ordering::Relaxed);
        mark("targets", t0);

        // Controllers (fan / AIO / RGB / direct-color writer)
        self.controllers.shutdown();
        mark("controllers", t0);

        // Drop RGB controller reference from IPC state before clearing the
        // device registry so device handles are released cleanly.
        self.ipc.state.lock().rgb_controller = None;
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

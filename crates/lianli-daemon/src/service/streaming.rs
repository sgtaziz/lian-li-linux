use super::ServiceManager;

impl ServiceManager {
    // Frame pushing is now handled by the dedicated streaming thread spawned
    // in run(). The main loop forwards FrameFinished events to the streaming
    // channel instead of calling send_frame synchronously.
}

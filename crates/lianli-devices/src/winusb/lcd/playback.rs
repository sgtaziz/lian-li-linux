use anyhow::Result;

#[derive(Debug, thiserror::Error)]
#[error("LCD packet transfer was interrupted")]
pub struct LcdTransferInterrupted;

#[derive(Debug, thiserror::Error)]
#[error("LCD handle is stale; re-discovery required")]
pub struct LcdRediscoveryRequired;

#[derive(Default)]
pub(super) enum PlaybackState {
    #[default]
    Unknown,
    Stopped,
    Playing,
    Interrupted,
}

impl PlaybackState {
    pub(super) fn ensure_ready(&self) -> Result<()> {
        if matches!(self, Self::Interrupted) {
            return Err(LcdTransferInterrupted.into());
        }
        Ok(())
    }

    pub(super) fn write(&mut self, write: impl FnOnce() -> Result<()>) -> Result<()> {
        self.ensure_ready()?;
        self.resend(write)
    }

    /// Bypasses the interrupted latch. Only for replaying a whole packet after
    /// the handle was closed and reopened, as the vendor SendAndRead does.
    pub(super) fn resend(&mut self, write: impl FnOnce() -> Result<()>) -> Result<()> {
        // A failed transfer may have delivered a header or part of its payload.
        *self = Self::Interrupted;
        write()?;
        *self = Self::Playing;
        Ok(())
    }

    pub(super) fn stop(&mut self, stop: impl FnOnce() -> Result<()>) -> Result<()> {
        self.ensure_ready()?;
        if !matches!(self, Self::Stopped) {
            stop()?;
            *self = Self::Stopped;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    #[test]
    fn attaching_and_replacing_playback_stop_each_session_once() {
        let mut state = PlaybackState::default();
        let mut stops = 0;
        state
            .stop(|| {
                stops += 1;
                Ok(())
            })
            .unwrap();
        state
            .stop(|| {
                stops += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(stops, 1);
        state.write(|| Ok(())).unwrap();
        state
            .stop(|| {
                stops += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(stops, 2);
    }

    #[test]
    fn interrupted_payload_is_neither_replayed_nor_followed_by_a_command() {
        let mut state = PlaybackState::default();
        let mut delivered = vec![];
        assert!(state
            .write(|| {
                delivered.extend_from_slice(b"header and partial payload");
                bail!("USB transfer failed")
            })
            .is_err());
        assert!(state
            .write(|| {
                delivered.extend_from_slice(b"replay");
                Ok(())
            })
            .is_err());
        assert!(state
            .stop(|| {
                delivered.extend_from_slice(b"stop");
                Ok(())
            })
            .is_err());
        assert_eq!(delivered, b"header and partial payload");
    }

    #[test]
    fn successful_resend_after_reopen_resumes_streaming_and_teardown() {
        let mut state = PlaybackState::default();
        assert!(state.write(|| bail!("USB transfer timed out")).is_err());
        assert!(state.resend(|| bail!("still stalled")).is_err());
        assert!(state
            .write(|| Ok(()))
            .unwrap_err()
            .is::<LcdTransferInterrupted>());
        let mut delivered = vec![];
        state
            .resend(|| {
                delivered.push("whole packet");
                Ok(())
            })
            .unwrap();
        state
            .write(|| {
                delivered.push("next packet");
                Ok(())
            })
            .unwrap();
        state
            .stop(|| {
                delivered.push("stop");
                Ok(())
            })
            .unwrap();
        assert_eq!(delivered, ["whole packet", "next packet", "stop"]);
    }

    #[test]
    fn failed_stop_is_not_reported_as_stopped() {
        let mut state = PlaybackState::default();
        assert!(state.stop(|| bail!("StopPlay failed")).is_err());
        let mut retried = false;
        state
            .stop(|| {
                retried = true;
                Ok(())
            })
            .unwrap();
        assert!(retried);
    }
}

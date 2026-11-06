pub(super) type RingKey = (Vec<u8>, u8);

#[derive(Default)]
pub(super) struct RingDelivery {
    pub applied: Option<RingKey>,
    pub attempted: Option<RingKey>,
    pub in_flight: Vec<RingKey>,
    written: bool,
}

impl RingDelivery {
    pub fn begin(&mut self, key: RingKey) {
        self.applied = None;
        self.attempted = Some(key);
        self.written = false;
    }

    pub fn written(&mut self) {
        self.written = true;
    }

    pub fn recovered(&mut self) {
        self.applied = self.attempted.take().filter(|_| self.written);
        self.written = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_write_is_not_applied_until_recovery_and_partial_write_is_not_deduplicated() {
        let mut state = RingDelivery::default();
        let key = (vec![1, 2, 3], 20);
        state.begin(key.clone());
        assert!(state.applied.is_none());
        state.written();
        assert_eq!(state.attempted, Some(key.clone()));
        assert!(state.applied.is_none());
        state.recovered();
        assert_eq!(state.applied, Some(key.clone()));
        assert!(state.attempted.is_none());
        state.begin(key);
        state.recovered();
        assert!(state.applied.is_none());
        assert!(state.attempted.is_none());
    }
}

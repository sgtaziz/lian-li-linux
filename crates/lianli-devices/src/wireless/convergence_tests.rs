use super::*;

#[test]
fn command_sequence_wrap_does_not_reuse_targets_while_acknowledgement_is_stale() {
    assert_eq!(next_target_sequence(None, 253), 254);
    assert_eq!(next_target_sequence(Some(254), 253), 1);
    assert_eq!(next_target_sequence(Some(1), 253), 2);
    assert_eq!(next_target_sequence(Some(2), 253), 3);
    assert_eq!(next_target_sequence(None, 255), 1);
    assert_eq!(next_target_sequence(None, 0), 1);
}

fn command(mac: u8, pwm: u8, queued_at: Instant) -> PendingCommand {
    PendingCommand {
        mac: [mac; 6],
        channel: 8,
        rx_type: 1,
        rf_data: vec![0; RF_DATA_SIZE],
        ack: AckSignal::Pwm([pwm; 4]),
        remaining_retries: INITIAL_RETRIES,
        last_sent: queued_at,
        queued_at,
        description: "PWM".into(),
    }
}

#[test]
fn rgb_quiet_period_does_not_disable_cooling_retries() {
    let pwm = AckSignal::Pwm([100; 4]);
    assert!(retry_due(Duration::from_millis(100), false, &pwm));
    assert!(!retry_due(Duration::from_millis(100), true, &pwm));
    assert!(!retry_due(Duration::from_millis(999), true, &pwm));
    assert!(retry_due(Duration::from_secs(1), true, &pwm));
    assert!(retry_due(
        Duration::from_millis(100),
        true,
        &AckSignal::CmdSeq(1)
    ));
}

#[test]
fn newer_pwm_replaces_queued_target_and_supersedes_inflight_retry() {
    let now = Instant::now();
    let mut old = command(1, 50, now);
    let new = command(1, 255, now + Duration::from_millis(1));
    let mut queue = VecDeque::new();
    admit_command(&mut queue, old.clone()).unwrap();
    admit_command(&mut queue, new.clone()).unwrap();
    assert_eq!(queue.len(), 1);
    assert!(matches!(queue[0].ack, AckSignal::Pwm([255, 255, 255, 255])));
    old.last_sent = now + Duration::from_secs(1);
    assert!(superseded_command(&queue, &old));
    assert!(!superseded_command(&queue, &new));
}

#[test]
fn mb_sync_off_supersedes_pending_on_without_discarding_cooling() {
    let now = Instant::now();
    let mut old = command(1, 0, now);
    old.ack = AckSignal::CmdSeq(1);
    old.rf_data[1] = super::super::RF_MB_LIGHT_SYNC;
    old.rf_data[20] = 1;
    let mut new = old.clone();
    new.ack = AckSignal::CmdSeq(2);
    new.rf_data[20] = 0;
    new.queued_at = now + Duration::from_millis(1);
    let mut queue = VecDeque::new();
    admit_command(&mut queue, command(1, 200, now)).unwrap();
    admit_command(&mut queue, old.clone()).unwrap();
    admit_command(&mut queue, new).unwrap();
    assert_eq!(queue.len(), 2);
    assert!(matches!(queue[0].ack, AckSignal::Pwm(_)));
    assert_eq!(queue[1].rf_data[20], 0);
    assert!(superseded_command(&queue, &old));
}

#[test]
fn full_queue_accepts_replacement_but_rejects_another_command_without_losing_work() {
    let now = Instant::now();
    let mut queue = VecDeque::new();
    for mac in 0..MAX_PENDING_COMMANDS {
        admit_command(&mut queue, command(mac as u8, 100, now)).unwrap();
    }
    let mut extra = command(1, 0, now);
    extra.ack = AckSignal::CmdSeq(1);
    assert!(admit_command(&mut queue, extra).is_err());
    assert_eq!(queue.len(), MAX_PENDING_COMMANDS);
    admit_command(&mut queue, command(1, 255, now + Duration::from_millis(1))).unwrap();
    assert_eq!(queue.len(), MAX_PENDING_COMMANDS);
    assert!(matches!(
        queue.back().unwrap().ack,
        AckSignal::Pwm([255, 255, 255, 255])
    ));
}

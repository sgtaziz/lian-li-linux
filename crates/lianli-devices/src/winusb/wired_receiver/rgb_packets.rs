use super::*;

pub(super) fn checked_frame_count(frame_count: usize) -> Result<u16> {
    anyhow::ensure!(frame_count > 0, "RGB upload requires at least one frame");
    u16::try_from(frame_count).context("RGB upload exceeds the protocol frame count")
}

pub(super) fn rgb_flash_header(
    compressed_len: usize,
    led_total: usize,
    effect_index: u32,
    total_frame: u16,
    timing: RgbPlaybackTiming,
) -> Result<[u8; PACKET_SIZE]> {
    anyhow::ensure!(
        total_frame > 0 && (1..=255).contains(&led_total),
        "invalid RGB upload layout"
    );
    anyhow::ensure!(
        if timing.secondary_frame_count == 0 {
            timing.secondary_interval_ticks == 0 && !timing.outer_longest
        } else {
            timing.secondary_interval_ticks > 0 && timing.secondary_frame_count <= total_frame
        },
        "invalid secondary RGB region timing"
    );
    anyhow::ensure!(
        (100..=6_553_599).contains(&timing.interval_hundredths),
        "RGB interval must be 1..=65535.99 ticks"
    );
    let interval_ticks = (timing.interval_hundredths / 100) as u16;
    let interval_fraction = (timing.interval_hundredths % 100) as u8;
    let mut header = [0u8; PACKET_SIZE];
    header[0] = CMD_SEND_LIGHT_PACKAGE;
    header[16..20].copy_from_slice(&effect_index.to_be_bytes());
    header[22..26].copy_from_slice(&(compressed_len as u32).to_be_bytes());
    header[27..29].copy_from_slice(&total_frame.to_be_bytes());
    header[29] = led_total as u8;
    header[34..36].copy_from_slice(&interval_ticks.to_be_bytes());
    header[36] = interval_fraction;
    header[37..39].copy_from_slice(&timing.secondary_interval_ticks.to_be_bytes());
    header[39] = u8::from(timing.outer_longest);
    header[40..42].copy_from_slice(&timing.secondary_frame_count.to_be_bytes());
    Ok(header)
}

pub(super) fn rgb_timeout(deadline: Instant, configured: Duration) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    anyhow::ensure!(!remaining.is_zero(), "wired RGB package deadline exceeded");
    Ok(remaining.min(configured))
}

pub(super) fn read_rgb_ack(
    transport: &RusbBulk,
    deadline: Instant,
    expected_command: u8,
) -> Result<()> {
    let mut rx = [0u8; PACKET_SIZE];
    let timeout = rgb_timeout(deadline, LCD_READ_TIMEOUT)?;
    transport
        .read(&mut rx, timeout)
        .context("wired RGB package ACK read")?;
    validate_rgb_ack(&rx, expected_command)
}

pub(super) fn validate_rgb_ack(response: &[u8], expected_command: u8) -> Result<()> {
    anyhow::ensure!(!response.is_empty(), "wired RGB package ACK is empty");
    anyhow::ensure!(
        response[0] == expected_command,
        "wired RGB package ACK command 0x{:02x}, expected 0x{:02x}",
        response[0],
        expected_command
    );
    Ok(())
}

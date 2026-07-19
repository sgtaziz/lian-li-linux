// Report IDs
pub(super) const REPORT_ID_A: u8 = 0x01;
pub(super) const REPORT_ID_B: u8 = 0x02;
pub(super) const REPORT_ID_C: u8 = 0x03;

// Packet sizes
pub(super) const A_PACKET_SIZE: usize = 64;
pub(super) const A_HEADER_LEN: usize = 6;
pub(super) const B_PACKET_SIZE: usize = 1024;
pub(super) const B_HEADER_LEN: usize = 11;
pub(super) const B_MAX_PAYLOAD: usize = B_PACKET_SIZE - B_HEADER_LEN; // 1013
pub(super) const C_PACKET_SIZE: usize = 512;
pub(super) const C_MAX_PAYLOAD: usize = C_PACKET_SIZE - 11; // 501

pub(super) const READ_TIMEOUT_MS: i32 = 1000;
pub(super) const INIT_READ_TIMEOUT_MS: i32 = 3000;
pub(super) const ACK_TIMEOUT_MS: i32 = 20;

// A-Commands. Currently-unused commands prefixed with `_`.
pub(super) const CMD_HANDSHAKE: u8 = 0x81;
pub(super) const CMD_SET_PUMP_LIGHT: u8 = 0x83;
pub(super) const _CMD_GET_FANLIGHT_INFO: u8 = 0x84;
pub(super) const CMD_SET_FAN_LIGHT: u8 = 0x85;
pub(super) const CMD_GET_FIRMWARE: u8 = 0x86;
pub(super) const _CMD_GET_SERIAL_NUMBER: u8 = 0x88;
pub(super) const _CMD_SET_SERIAL_NUMBER: u8 = 0x89;
pub(super) const CMD_SET_PUMP_PWM: u8 = 0x8A;
pub(super) const CMD_SET_FAN_PWM: u8 = 0x8B;
pub(super) const CMD_RESET_DEVICE: u8 = 0x8E;

// B-Commands
pub(super) const CMD_LCD_CONTROL: u8 = 0x0C;
pub(super) const CMD_SEND_H264: u8 = 0x0D;
pub(super) const CMD_SEND_JPEG: u8 = 0x0E;
pub(super) const CMD_LCD_AVAILABLE: u8 = 0x17;

pub(super) const FAN_LED_COUNT: u16 = 24;

pub(super) use lianli_shared::fan::duty_to_percent;

/// Build a B-command (1024B) or C-command (512B) LCD packet.
/// Header layout is identical; only report ID, packet size, and max payload differ.
pub(super) fn build_lcd_packet(
    report_id: u8,
    pkt_size: usize,
    cmd: u8,
    total_data_size: u32,
    packet_num: u32,
    payload: &[u8],
) -> Vec<u8> {
    let header_len = 11;
    let max_payload = pkt_size - header_len;
    let mut pkt = vec![0u8; pkt_size];

    pkt[0] = report_id;
    pkt[1] = cmd;

    pkt[2] = (total_data_size >> 24) as u8;
    pkt[3] = (total_data_size >> 16) as u8;
    pkt[4] = (total_data_size >> 8) as u8;
    pkt[5] = total_data_size as u8;

    pkt[6] = (packet_num >> 16) as u8;
    pkt[7] = (packet_num >> 8) as u8;
    pkt[8] = packet_num as u8;

    let len = payload.len().min(max_payload);
    pkt[9] = (len >> 8) as u8;
    pkt[10] = len as u8;

    if len > 0 {
        pkt[header_len..header_len + len].copy_from_slice(&payload[..len]);
    }

    pkt
}

/// Parse firmware version from strings like "1.2", "V1.3", or
/// "N9,01,HS,SQ,HydroShift,V3.0C.013,1.3". Scans for the last "major.minor"
/// pattern where both are numeric.
pub(super) fn parse_firmware_version(fw: &str) -> Option<(u32, u32)> {
    for segment in fw.rsplit(',') {
        let s = segment
            .trim()
            .trim_start_matches(|c: char| !c.is_ascii_digit());
        let mut parts = s.split('.');
        if let (Some(maj_s), Some(min_s)) = (parts.next(), parts.next()) {
            if let (Ok(major), Ok(minor)) = (maj_s.parse::<u32>(), min_s.parse::<u32>()) {
                return Some((major, minor));
            }
        }
    }
    None
}

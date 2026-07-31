use cbc::Encryptor;
use des::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use des::Des;
use std::time::Instant;

const DES_KEY: [u8; 8] = *b"slv3tuzx";

type DesCbc = Encryptor<Des>;

// Command types for VID=0x1CBE LCD devices.
pub const CMD_GET_VER: u8 = 0x0A;
pub const CMD_ROTATE: u8 = 0x0D;
pub const CMD_BRIGHTNESS: u8 = 0x0E;
pub const CMD_FRAME_RATE: u8 = 0x0F;
pub const CMD_SET_CLOCK: u8 = 0x33;
pub const CMD_STOP_CLOCK: u8 = 0x34;
pub const CMD_PUSH_JPG: u8 = 0x65;
pub const CMD_PUSH_PNG: u8 = 0x66;
pub const CMD_START_PLAY: u8 = 0x79;
pub const CMD_QUERY_BLOCK: u8 = 0x7A;
pub const CMD_STOP_PLAY: u8 = 0x7B;
pub const CMD_REBOOT: u8 = 0x0B;
pub const CMD_SWITCH_TO_DESKTOP: u8 = 0x96;
pub const CMD_WARN_SWITCH: u8 = 0x2E;

// HydroShift II AIO opcodes
pub const CMD_GET_H2_PARAMS: u8 = 0xFA;
pub const CMD_SYNC_PUMP_FAN: u8 = 0xFB;
pub const CMD_PUSH_RGB_DATA: u8 = 0xFC;
pub const CMD_SET_WTHEME_INDEX: u8 = 0xF9;

/// CRC-16/CCITT (poly 0x1021, init 0, no final XOR, no reflection).
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Builds DES-CBC encrypted command headers for VID=0x1CBE LCD devices.
///
/// All VID=0x1CBE devices (SLV3, TLV2, HydroShift II, Lancool 207, Universal Screen)
/// share this exact same encrypted 512-byte header format.
pub struct PacketBuilder {
    start_time: Instant,
    last_timestamp: u32,
}

impl PacketBuilder {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            last_timestamp: 0,
        }
    }

    /// Build a 512-byte encrypted header with raw parameter bytes at offset 8+.
    fn build(&mut self, command: u8, params: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 504 + 8];
        buf[0] = command;
        buf[2] = 0x1A;
        buf[3] = 0x6D;

        let raw = self.start_time.elapsed().as_millis() as u32;
        let ts = if raw <= self.last_timestamp {
            self.last_timestamp + 1
        } else {
            raw
        };
        self.last_timestamp = ts;
        buf[4..8].copy_from_slice(&ts.to_le_bytes());

        let copy_len = params.len().min(496);
        buf[8..8 + copy_len].copy_from_slice(&params[..copy_len]);

        let cipher = DesCbc::new_from_slices(&DES_KEY, &DES_KEY)
            .expect("DES key and IV must both be 8 bytes");
        cipher
            .encrypt_padded_mut::<Pkcs7>(&mut buf, 504)
            .expect("padding")
            .to_vec()
    }

    /// Build a 512-byte encrypted header.
    ///
    /// - `payload_size`: size of the JPEG payload that follows
    /// - `command`: command byte (e.g., CMD_PUSH_JPG)
    /// - `include_size`: whether to embed the payload size at offset 8
    pub fn header(&mut self, payload_size: usize, command: u8, include_size: bool) -> Vec<u8> {
        if include_size {
            self.build(command, &(payload_size as u32).to_be_bytes())
        } else {
            self.build(command, &[])
        }
    }

    /// Build a JPEG frame header (cmd 0x65 with payload size).
    pub fn jpeg_header(&mut self, jpeg_size: usize) -> Vec<u8> {
        self.header(jpeg_size, CMD_PUSH_JPG, true)
    }

    /// Build a brightness control header (cmd 0x0E, value 0-100).
    pub fn brightness_header(&mut self, brightness: u8) -> Vec<u8> {
        self.build(CMD_BRIGHTNESS, &[brightness.min(100)])
    }

    /// Build a rotation control header (cmd 0x0D, value 0-3).
    pub fn rotation_header(&mut self, rotation: u8) -> Vec<u8> {
        self.build(CMD_ROTATE, &[rotation & 0x03])
    }

    /// Build a frame rate control header (cmd 0x0F).
    pub fn frame_rate_header(&mut self, fps: u8) -> Vec<u8> {
        self.build(CMD_FRAME_RATE, &[fps])
    }

    // WinUSB packet format: 500-byte plaintext -> DES-CBC-PKCS7 -> 504 bytes,
    // placed in a 512-byte frame with trailer [510]=0xa1, [511]=0x1a.
    fn build_winusb(&mut self, command: u8, params: &[u8]) -> Vec<u8> {
        // 500-byte plaintext; need 500 + block_size(8) bytes for encrypt_padded_mut
        let mut buf = vec![0u8; 508];
        buf[0] = command;
        buf[2] = 0x1A;
        buf[3] = 0x6D;

        let raw = self.start_time.elapsed().as_millis() as u32;
        let ts = if raw <= self.last_timestamp {
            self.last_timestamp + 1
        } else {
            raw
        };
        self.last_timestamp = ts;
        buf[4..8].copy_from_slice(&ts.to_le_bytes());

        let copy_len = params.len().min(492);
        buf[8..8 + copy_len].copy_from_slice(&params[..copy_len]);

        // Encrypt the first 500 bytes: PKCS7 pads 500 -> 504 bytes (adds 4 bytes)
        let cipher = DesCbc::new_from_slices(&DES_KEY, &DES_KEY)
            .expect("DES key and IV must both be 8 bytes");
        let encrypted = cipher
            .encrypt_padded_mut::<Pkcs7>(&mut buf, 500)
            .expect("padding")
            .to_vec();
        // encrypted.len() == 504

        // Build the 512-byte header: encrypted + zeros + trailer
        let mut out = vec![0u8; 512];
        out[..504].copy_from_slice(&encrypted);
        out[510] = 0xa1;
        out[511] = 0x1a;
        out
    }

    pub fn jpeg_header_winusb(&mut self, jpeg_size: usize) -> Vec<u8> {
        self.build_winusb(CMD_PUSH_JPG, &(jpeg_size as u32).to_be_bytes())
    }

    pub fn png_header_winusb(&mut self, png_size: usize) -> Vec<u8> {
        self.build_winusb(CMD_PUSH_PNG, &(png_size as u32).to_be_bytes())
    }

    pub fn frame_rate_header_winusb(&mut self, fps: u8) -> Vec<u8> {
        self.build_winusb(CMD_FRAME_RATE, &[fps])
    }

    pub fn rotation_header_winusb(&mut self, rotation: u8) -> Vec<u8> {
        self.build_winusb(CMD_ROTATE, &[rotation & 0x03])
    }

    pub fn brightness_header_winusb(&mut self, brightness: u8) -> Vec<u8> {
        self.build_winusb(CMD_BRIGHTNESS, &[brightness.min(100)])
    }

    pub fn get_ver_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_GET_VER, &[])
    }

    pub fn start_play_header_winusb(&mut self, chunk_len: usize, is_last: bool) -> Vec<u8> {
        let play_tick = self.start_time.elapsed().as_millis() as u32;
        let mut params = (chunk_len as u32).to_be_bytes().to_vec();
        params.push(if is_last { 1 } else { 0 });
        params.push(0);
        params.extend_from_slice(&play_tick.to_be_bytes());
        self.build_winusb(CMD_START_PLAY, &params)
    }

    pub fn query_block_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_QUERY_BLOCK, &[])
    }

    pub fn stop_play_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_STOP_PLAY, &[])
    }

    pub fn sync_clock_header_winusb(&mut self, mode: u8) -> Vec<u8> {
        let now = time::OffsetDateTime::now_utc();
        let y = now.year() as u16;
        self.build_winusb(
            CMD_SET_CLOCK,
            &[
                (y >> 8) as u8,
                (y & 0xFF) as u8,
                now.month() as u8,
                now.day(),
                now.hour(),
                now.minute(),
                now.second(),
                mode,
            ],
        )
    }

    pub fn stop_clock_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_STOP_CLOCK, &[0])
    }

    pub fn switch_to_desktop_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_SWITCH_TO_DESKTOP, &[])
    }

    pub fn reboot_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_REBOOT, &[])
    }

    pub fn warn_switch_header_winusb(&mut self, is_open: bool) -> Vec<u8> {
        self.build_winusb(CMD_WARN_SWITCH, &[if is_open { 1 } else { 0 }])
    }

    /// Build a SyncPumpFan header (cmd 0xFB) with CRC16.
    pub fn sync_pump_fan_header_winusb(
        &mut self,
        pump_pwm: u16,
        fan1: u8,
        fan2: u8,
        fan3: u8,
    ) -> Vec<u8> {
        let mut payload = [0u8; 492];
        payload[0] = 0xFF;
        payload[1] = 0x0F;
        payload[2] = 0xA2;
        payload[3] = 0x00;
        payload[4] = (pump_pwm >> 8) as u8;
        payload[5] = (pump_pwm & 0xFF) as u8;
        payload[6] = fan1;
        payload[7] = fan2;
        payload[8] = fan3;
        let crc = crc16_ccitt(&payload[..12]);
        payload[12] = (crc >> 8) as u8;
        payload[13] = (crc & 0xFF) as u8;
        self.build_winusb(CMD_SYNC_PUMP_FAN, &payload)
    }

    /// Build a GetH2Params header (cmd 0xFA) — telemetry read.
    pub fn get_h2_params_header_winusb(&mut self) -> Vec<u8> {
        self.build_winusb(CMD_GET_H2_PARAMS, &[])
    }

    /// PushRgbData header (cmd 0xFC). Payload length is at offset 12, not 8.
    pub fn push_rgb_data_header_winusb(&mut self, payload_len: usize) -> Vec<u8> {
        let mut params = [0u8; 8];
        params[4..8].copy_from_slice(&(payload_len as u32).to_be_bytes());
        self.build_winusb(CMD_PUSH_RGB_DATA, &params)
    }
}

impl Default for PacketBuilder {
    fn default() -> Self {
        Self::new()
    }
}

const LGBL_KEY: &[u8; 8] = b"lgbm9lgb";

pub fn decrypt_lgbl(input: &[u8]) -> Vec<u8> {
    input
        .iter()
        .enumerate()
        .map(|(i, &b)| b ^ LGBL_KEY[i % 8])
        .collect()
}

pub fn is_lgbl(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"lgbl"
}

#[cfg(test)]
mod tests {
    use super::{crc16_ccitt, decrypt_lgbl, is_lgbl};

    #[test]
    fn crc16_empty_is_zero() {
        assert_eq!(crc16_ccitt(&[]), 0);
    }

    #[test]
    fn crc16_single_byte_matches_table() {
        // table[1] = 4129 = 0x1021
        assert_eq!(crc16_ccitt(&[0x01]), 0x1021);
    }

    #[test]
    fn crc16_known_vector() {
        // CRC-16/XMODEM of "123456789" = 0x31C3
        assert_eq!(crc16_ccitt(b"123456789"), 0x31C3);
    }

    #[test]
    fn lgbl_decrypt_round_trip() {
        let original = b"Hello, World!";
        let encrypted: Vec<u8> = original
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ b"lgbm9lgb"[i % 8])
            .collect();
        let decrypted = decrypt_lgbl(&encrypted);
        assert_eq!(&decrypted[..], original);
    }

    #[test]
    fn lgbl_detection() {
        assert!(is_lgbl(b"lgbl\x00\x01"));
        assert!(!is_lgbl(b"other"));
        assert!(!is_lgbl(b"lg"));
    }

    #[test]
    fn lgbl_magic_xors_to_h264_start_code() {
        let encrypted = b"lgbl";
        let decrypted = decrypt_lgbl(encrypted);
        assert_eq!(&decrypted[..4], &[0x00, 0x00, 0x00, 0x01]);
    }
}

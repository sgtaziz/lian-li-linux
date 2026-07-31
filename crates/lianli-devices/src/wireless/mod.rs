//! Wireless RF dongle driver — TX/RX dongles + bound wireless fans/AIOs/strips.

pub mod adapters;
mod aio;
mod bind;
mod clock_sync;
mod controller;
mod convergence;
mod discovery;
mod fan_speed;
mod fan_type;
mod mb_sync;
mod opcodes;
mod rgb;
mod transport;
mod v2_hid;

pub use aio::pump_rpm_to_timer;
pub use clock_sync::{build_payload, SensorSnapshot};
pub use controller::WirelessController;
pub use discovery::DiscoveredDevice;
pub use fan_type::WirelessFanType;
pub use v2_hid::{query_v2_hid_macs, share_parent, V2HidEntry, V2_HID_PID, V2_HID_VID};

use once_cell::sync::Lazy;

/// TX dongle VID:PID pairs (V1 and V2 hardware).
const TX_IDS: [(u16, u16); 2] = [(0x0416, 0x8040), (0x1A86, 0xE304)];
/// RX dongle VID:PID pairs (V1 and V2 hardware).
const RX_IDS: [(u16, u16); 2] = [(0x0416, 0x8041), (0x1A86, 0xE305)];

pub fn tx_dongle_present() -> bool {
    let Ok(devices) = rusb::devices() else {
        return false;
    };
    for device in devices.iter() {
        if let Ok(desc) = device.device_descriptor() {
            let id = (desc.vendor_id(), desc.product_id());
            if TX_IDS.contains(&id) {
                return true;
            }
        }
    }
    false
}

const USB_CMD_SEND_RF: u8 = 0x10;
const USB_CMD_GET_MAC: u8 = 0x11;

const RF_SELECT: u8 = 0x12;
const RF_PWM_CMD: u8 = 0x10;
const RF_SELECTED_GROUP: u8 = 0x12;
const RF_CLOCK_SYNC: u8 = 0x14;
const RF_REBOOT_LCD: u8 = 0x16;
const RF_AIO_SWITCH_WIRELESS: u8 = 0x19;
const RF_SET_RGB: u8 = 0x20;
const RF_SEND_PIC: u8 = 0x22;
const RF_217_CLOSE_WIFI: u8 = 0x23;
const RF_AIO_PARAMS: u8 = 0x21;
const RF_MB_LIGHT_SYNC: u8 = 0x27;

const RF_DATA_SIZE: usize = 240;
const RF_CHUNK_SIZE: usize = 60;
const RF_CHUNKS: usize = RF_DATA_SIZE / RF_CHUNK_SIZE;

/// Size of the aio_param state block sent over RF to wireless AIOs.
pub const AIO_PARAM_LEN: usize = 32;

static CMD_RESET: Lazy<Vec<u8>> = Lazy::new(|| decode_command("11080000"));
static CMD_VIDEO_START: Lazy<Vec<u8>> = Lazy::new(|| decode_command("11010000"));
static CMD_RX_QUERY_34: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010434"));
static CMD_RX_QUERY_37: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010437"));
static CMD_RX_LCD_MODE: Lazy<Vec<u8>> = Lazy::new(|| decode_command("10010430"));

fn decode_command(prefix: &str) -> Vec<u8> {
    let mut bytes = hex::decode(prefix).expect("valid hex literal");
    bytes.resize(64, 0u8);
    bytes
}

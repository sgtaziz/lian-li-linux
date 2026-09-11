use super::{controller::WirelessController, USB_CMD_GET_MAC};
use anyhow::{ensure, Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use std::time::{Duration, Instant};

impl WirelessController {
    pub fn read_rgb_clock(&self) -> Result<(u32, Instant)> {
        let master = *self.master_mac.lock();
        let mut request = [0; 64];
        request[0] = USB_CMD_GET_MAC;
        request[1] = *self.master_channel.lock();
        self.tx_recover(|transport| {
            transport
                .write(&request, USB_TIMEOUT)
                .context("querying RGB master clock")?;
            let mut response = [0; 64];
            let len = transport.read(&mut response, Duration::from_millis(500))?;
            let received = Instant::now();
            Ok((parse_clock(&response[..len], master)?, received))
        })
    }
}

fn parse_clock(response: &[u8], master: [u8; 6]) -> Result<u32> {
    ensure!(
        response.len() >= 11 && response[0] == USB_CMD_GET_MAC && response[1..7] == master,
        "invalid RGB master clock response"
    );
    Ok(u32::from_be_bytes(response[7..11].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_response_checks_master_identity_length_and_big_endian_ticks() {
        let response = [0x11, 1, 2, 3, 4, 5, 6, 0x12, 0x34, 0x56, 0x78];
        assert_eq!(
            parse_clock(&response, [1, 2, 3, 4, 5, 6]).unwrap(),
            0x12345678
        );
        assert!(parse_clock(&response[..10], [1, 2, 3, 4, 5, 6]).is_err());
        assert!(parse_clock(&response, [0; 6]).is_err());
    }
}

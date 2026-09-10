use super::*;

impl WiredReceiverController {
    /// SetFansPWM (0x13) with per-device PWM floor.
    pub fn set_fans_pwm(&self, duties: [u8; 4]) -> Result<()> {
        if self.is_wireless.load(Ordering::Relaxed) {
            return Ok(());
        }
        let fan_count = *self.fan_count.lock() as usize;
        let right_attach = *self.is_inf_right_attach.lock();
        let mut duties = duties;
        if right_attach {
            reverse_fan_slots(&mut duties, fan_count);
        }
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_FANS_PWM;
        for i in 0..4 {
            let p = duties[i] as u16;
            // Per-device floor: P28 max(p,8)/zero→1; TL Flex max(p,11)/zero→5
            let mapped = if p == 0 {
                self.params.pwm_zero
            } else {
                ((p.max(self.params.pwm_floor as u16) as f64 / 100.0) * 255.0).round() as u8
            };
            // 0x06 → 0x00 rewrite (external-sync sentinel)
            tx[1 + i] = if mapped == 0x06 { 0 } else { mapped };
        }
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_FANS_PWM || rx[1] != 0 {
            warn!("SetFansPWM unexpected response: [{}, {}]", rx[0], rx[1]);
        }
        debug!("{}: SetFansPWM {:?}", self.params.name, &tx[1..5]);
        Ok(())
    }

    /// Enable/disable motherboard PWM sync. Sentinels all four fan ports
    /// to value 6, which the firmware interprets as "follow MB PWM header."
    pub fn set_mb_sync(&self, enabled: bool) -> Result<()> {
        if self.is_wireless.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_FANS_PWM;
        if enabled {
            tx[1] = 6;
            tx[2] = 6;
            tx[3] = 6;
            tx[4] = 6;
        } else {
            return Ok(());
        }
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_FANS_PWM || rx[1] != 0 {
            warn!("MB sync unexpected response: [{}, {}]", rx[0], rx[1]);
        }
        debug!("{}: MB PWM sync = {enabled}", self.params.name);
        Ok(())
    }

    /// SelectedGroup (0x16) — group selection / keepalive.
    pub fn selected_group(&self) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SELECTED_GROUP;
        self.send_and_read(&tx)?;
        Ok(())
    }

    /// SetLightSyncMB (0x14) — enable/disable motherboard RGB sync.
    pub fn set_light_sync_mb(&self, enable: bool) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_LIGHT_SYNC_MB;
        tx[1] = if enable { 1 } else { 0 };
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_LIGHT_SYNC_MB {
            warn!("SetLightSyncMB unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// SaveOrClearConfig (0x15) — save current RGB to NVRAM, or clear.
    /// `save=true` writes to NVRAM; `save=false` clears saved config.
    pub fn save_or_clear_config(&self, save: bool) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SAVE_OR_CLEAR;
        tx[1] = if save { 1 } else { 0 };
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SAVE_OR_CLEAR {
            warn!("SaveOrClearConfig unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// RebootLcd (0x17) — reboot the wired LCD group.
    pub fn reboot_lcd(&self) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_REBOOT_LCD;
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_REBOOT_LCD && rx[0] != CMD_SAVE_OR_CLEAR {
            warn!("RebootLcd unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// FanAndFixedData (0x26) — per-fan theme/data/brightness push.
    /// `fans_data` is up to 62 bytes of per-fan configuration.
    pub fn update_fans_theme_and_data(&self, fans_data: &[u8]) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_FAN_AND_FIXED_DATA;
        let len = fans_data.len().min(62);
        tx[2..2 + len].copy_from_slice(&fans_data[..len]);
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_FAN_THEME_COLOR && rx[0] != CMD_FAN_AND_FIXED_DATA {
            warn!("FanAndFixedData unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// FanThemeColor (0x27) — per-fan color palette.
    /// `colors` is the RGB data; `fan_index` selects which fan (0-3).
    pub fn update_fans_color(&self, colors: &[[u8; 3]], fan_index: u8) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_FAN_THEME_COLOR;
        tx[1] = if fan_index > 1 { 1 } else { 0 };
        let start = 2 + (fan_index as usize % 2) * 19;
        let mut num = start;
        for c in colors.iter().take(6) {
            if num + 3 > PACKET_SIZE {
                break;
            }
            tx[num] = c[0];
            tx[num + 1] = c[1];
            tx[num + 2] = c[2];
            num += 3;
        }
        let mut nonce = self.color_nonce.lock();
        *nonce = nonce.wrapping_add(1).max(1);
        tx[num] = *nonce;
        tx[num + 1] = nonce.wrapping_add(1).max(1);
        drop(nonce);
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_FAN_THEME_COLOR {
            warn!("FanThemeColor unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }

    /// WirelessThemeSwitch (0x29) — toggle embedded theme vs USB frames.
    /// Bit `i` (0..3) = fan `i` uses embedded/wireless theme.
    pub fn update_wireless_theme_switch(&self, mask: u8) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_WIRELESS_THEME_SWITCH;
        tx[1] = mask;
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_WIRELESS_THEME_SWITCH {
            warn!("WirelessThemeSwitch unexpected response: 0x{:02x}", rx[0]);
        }
        Ok(())
    }
}

impl FanDevice for WiredReceiverController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = [0u8; 4];
        duties[slot as usize % 4] = duty;
        self.set_fans_pwm(duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut pwm = [0u8; 4];
        for (i, &d) in duties.iter().enumerate().take(4) {
            pwm[i] = lianli_shared::fan::duty_to_percent(d);
        }
        self.set_fans_pwm(pwm)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        if self.is_wireless.load(Ordering::Relaxed) {
            return Ok(Vec::new());
        }
        let status = self.get_info()?;
        Ok(status.fan_rpm.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        *self.fan_count.lock()
    }

    fn stop_pwm(&self) -> u8 {
        // zero duty maps to pwm_zero sentinel, not actual 0
        self.params.pwm_zero
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, _port: u8, sync: bool) -> Result<()> {
        self.set_mb_sync(sync)
    }

    fn wireless_link_mac(&self) -> Option<[u8; 6]> {
        *self.mac.lock()
    }

    fn set_wireless_bound(&self, bound: bool) {
        self.is_wireless.store(bound, Ordering::Relaxed);
    }
}

/// Reverse per-fan slot ordering for SL-INF right-attach daisy-chains.
fn reverse_fan_slots<T: Copy>(slots: &mut [T; 4], fan_count: usize) {
    let n = fan_count.min(4);
    if n > 1 {
        slots[..n].reverse();
    }
}

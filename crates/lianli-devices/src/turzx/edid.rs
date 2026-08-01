use super::vendor_caps::{Mode, VendorCaps};

pub fn patch_edid_serial(edid: &mut [u8; 128], serial: u32) {
    edid[12..16].copy_from_slice(&serial.to_le_bytes());
    let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
    edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
}

/// Build a DRM-valid 128-byte EDID 1.4 block from a device's vendor-descriptor
/// capabilities.
///
/// The device's own EDID ships with broken Detailed Timing Descriptors (H
/// sync pulse exceeds H blanking, etc.) that Linux/DRM rejects — so it
/// exposes zero modes on the connector and compositors can't mode-set.
/// We synthesize fresh DTDs using reduced-blanking CEA-style timings that
/// match the advertised pixel dimensions and refresh rates.
pub fn build_edid(caps: &VendorCaps, serial: u32) -> [u8; 128] {
    let mut edid = [0u8; 128];
    edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    // Manufacturer "TUR" (bytes 8-9): 3 letters × 5 bits, big-endian.
    edid[8] = 0x52;
    edid[9] = 0xB2;
    // Product code (LE) — distinct from device's 0x0000 so the synthetic EDID
    // is identifiable in logs.
    edid[10] = 0x01;
    edid[11] = 0x00;
    edid[12..16].copy_from_slice(&serial.to_le_bytes());
    edid[16] = 1;
    edid[17] = 35; // mfg year = 1990 + 35 = 2025
    edid[18] = 1;
    edid[19] = 4; // EDID 1.4

    edid[20] = 0xA5;
    edid[21] = 48;
    edid[22] = 12;
    edid[23] = 120;
    edid[24] = 0x06;
    edid[25..35].copy_from_slice(&[0xEE, 0x95, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]);
    edid[35] = 0;
    edid[36] = 0;
    edid[37] = 0;
    for i in 0..8 {
        edid[38 + i * 2] = 0x01;
        edid[38 + i * 2 + 1] = 0x01;
    }

    let mut modes: Vec<Mode> = Vec::new();
    for m in &caps.modes {
        if !modes
            .iter()
            .any(|u| u.width == m.width && u.height == m.height && u.refresh_hz == m.refresh_hz)
        {
            modes.push(*m);
        }
    }
    modes.sort_by(|a, b| {
        b.refresh_hz
            .cmp(&a.refresh_hz)
            .then((b.width as u32 * b.height as u32).cmp(&(a.width as u32 * a.height as u32)))
    });

    for slot in 0..4 {
        let off = 54 + slot * 18;
        if slot < modes.len() {
            let m = modes[slot];
            edid[off..off + 18].copy_from_slice(&build_dtd(m.width, m.height, m.refresh_hz));
        } else if slot == modes.len() {
            edid[off..off + 18].copy_from_slice(&build_monitor_name_descriptor("LianLi TURZX"));
        } else {
            edid[off..off + 5].copy_from_slice(&[0, 0, 0, 0x10, 0]);
        }
    }

    edid[126] = 0;
    let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
    edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
    edid
}

fn build_dtd(width: u16, height: u16, refresh_hz: u8) -> [u8; 18] {
    // Reduced-blanking timings, CEA-style. Safe for any compositor.
    const H_BLANK: u16 = 160;
    const H_FRONT: u16 = 48;
    const H_SYNC: u16 = 32;
    const V_BLANK: u16 = 45;
    const V_FRONT: u16 = 4;
    const V_SYNC: u16 = 5;

    let pixel_clock_khz =
        ((width as u32 + H_BLANK as u32) * (height as u32 + V_BLANK as u32) * refresh_hz as u32)
            / 1000;
    let pixel_clock_10khz = (pixel_clock_khz / 10).min(u16::MAX as u32) as u16;

    let mut dtd = [0u8; 18];
    dtd[..2].copy_from_slice(&pixel_clock_10khz.to_le_bytes());

    dtd[2] = (width & 0xFF) as u8;
    dtd[3] = (H_BLANK & 0xFF) as u8;
    dtd[4] = ((((width >> 8) & 0x0F) << 4) | ((H_BLANK >> 8) & 0x0F)) as u8;

    dtd[5] = (height & 0xFF) as u8;
    dtd[6] = (V_BLANK & 0xFF) as u8;
    dtd[7] = ((((height >> 8) & 0x0F) << 4) | ((V_BLANK >> 8) & 0x0F)) as u8;

    dtd[8] = (H_FRONT & 0xFF) as u8;
    dtd[9] = (H_SYNC & 0xFF) as u8;

    dtd[10] = ((((V_FRONT & 0x0F) << 4) | (V_SYNC & 0x0F)) & 0xFF) as u8;

    dtd[11] = ((((H_FRONT >> 8) & 0x03) << 6)
        | (((H_SYNC >> 8) & 0x03) << 4)
        | (((V_FRONT >> 4) & 0x03) << 2)
        | ((V_SYNC >> 4) & 0x03)) as u8;

    let h_mm: u16 = 0;
    let v_mm: u16 = 0;
    dtd[12] = (h_mm & 0xFF) as u8;
    dtd[13] = (v_mm & 0xFF) as u8;
    dtd[14] = ((((h_mm >> 8) & 0x0F) << 4) | ((v_mm >> 8) & 0x0F)) as u8;

    dtd[15] = 0;
    dtd[16] = 0;
    // Digital separate sync, positive H + V polarity, non-interlaced.
    dtd[17] = 0x1E;
    dtd
}

fn build_monitor_name_descriptor(name: &str) -> [u8; 18] {
    let mut d = [0u8; 18];
    d[3] = 0xFC; // type: monitor name
    let bytes = name.as_bytes();
    let n = bytes.len().min(13);
    d[5..5 + n].copy_from_slice(&bytes[..n]);
    for b in &mut d[5 + n..18] {
        // Line-feed padding, per EDID spec for name.
        *b = 0x0A;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_edid_writes_serial_and_fixes_checksum() {
        let mut edid = [0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[8] = 0x52;
        edid[9] = 0xB2;
        let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
        edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
        let sum_before: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum_before % 256, 0, "precondition");

        patch_edid_serial(&mut edid, 0xDEAD_BEEF);

        assert_eq!(&edid[12..16], &0xDEAD_BEEFu32.to_le_bytes());
        let sum_after: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum_after % 256, 0, "checksum still valid after patch");
    }

    #[test]
    fn synthetic_edid_is_valid() {
        let mut caps = VendorCaps {
            max_w: 2047,
            max_h: 2047,
            ..VendorCaps::default()
        };
        caps.modes.push(Mode {
            width: 1920,
            height: 480,
            refresh_hz: 60,
        });
        caps.modes.push(Mode {
            width: 1920,
            height: 480,
            refresh_hz: 30,
        });
        let edid = build_edid(&caps, 0xF73B_8A15);

        assert_eq!(
            &edid[..8],
            &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]
        );
        assert_eq!(&edid[8..10], &[0x52, 0xB2]);
        assert_eq!(&edid[12..16], &0xF73B_8A15u32.to_le_bytes());
        assert_eq!(edid[18], 1);
        assert_eq!(edid[19], 4);
        let sum: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum % 256, 0);

        let dtd = &edid[54..72];
        let px_clock_khz = u16::from_le_bytes([dtd[0], dtd[1]]) as u32 * 10;
        let h_active = (((dtd[4] as u16) & 0xF0) << 4) | dtd[2] as u16;
        let h_blank = (((dtd[4] as u16) & 0x0F) << 8) | dtd[3] as u16;
        let v_active = (((dtd[7] as u16) & 0xF0) << 4) | dtd[5] as u16;
        let v_blank = (((dtd[7] as u16) & 0x0F) << 8) | dtd[6] as u16;
        let h_sync_pulse = (((dtd[11] as u16) & 0x30) << 4) | dtd[9] as u16;
        let h_front = (((dtd[11] as u16) & 0xC0) << 2) | dtd[8] as u16;
        assert_eq!(h_active, 1920);
        assert_eq!(v_active, 480);
        assert!(
            h_front + h_sync_pulse < h_blank,
            "H sync pulse ({h_sync_pulse}) + front ({h_front}) must fit H blanking ({h_blank})"
        );
        assert!(v_blank > 0);
        assert!(
            (30_000..=150_000).contains(&px_clock_khz),
            "pixel clock {px_clock_khz} kHz outside sane range"
        );
    }
}

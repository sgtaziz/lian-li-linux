use super::{COMMIT, CTRL_OP, MAGIC, STREAM_A_FINAL, STREAM_A_FRAG};

pub fn tlv(buf: &mut Vec<u8>, sub_op: u8, value: u8) {
    buf.extend_from_slice(&[MAGIC, CTRL_OP, sub_op, value]);
}

pub fn build_config_packet(width: u16, height: u16, format: u16) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(28);
    tlv(&mut pkt, 0x00, 0x01);
    tlv(&mut pkt, 0x01, (width >> 8) as u8);
    tlv(&mut pkt, 0x02, width as u8);
    tlv(&mut pkt, 0x03, (height >> 8) as u8);
    tlv(&mut pkt, 0x04, height as u8);
    tlv(&mut pkt, format as u8, (format >> 8) as u8);
    tlv(&mut pkt, 0x1F, 0x01);
    pkt
}

pub fn build_power_off() -> [u8; 4] {
    [MAGIC, CTRL_OP, 0x1F, 0x02]
}

fn write_header(buf: &mut Vec<u8>, opcode: u8, offset: u32, size: u32) {
    buf.push(MAGIC);
    buf.push(opcode);
    buf.push(((offset >> 16) & 0xFF) as u8);
    buf.push(((offset >> 8) & 0xFF) as u8);
    buf.push((offset & 0xFF) as u8);
    buf.push(((size >> 16) & 0xFF) as u8);
    buf.push(((size >> 8) & 0xFF) as u8);
    buf.push((size & 0xFF) as u8);
}

pub const FRAME_OVERHEAD: usize = 10;

pub fn pack_frame(opcode: u8, offset: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(payload.len() + FRAME_OVERHEAD);
    write_header(&mut pkt, opcode, offset, payload.len() as u32);
    pkt.extend_from_slice(payload);
    pkt.push(MAGIC);
    pkt.push(COMMIT);
    pkt
}

pub fn pack_fragment(opcode: u8, offset: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(payload.len() + 8);
    write_header(&mut pkt, opcode, offset, payload.len() as u32);
    pkt.extend_from_slice(payload);
    pkt
}

/// Split a stream A payload into one or more URB-sized buffers following the
/// driver's fragmentation rules: intermediate fragments use opcode 0x6C with
/// no commit trailer; the last URB uses opcode 0x6D and carries the
/// `0xAF 0x66` commit marker.
pub fn fragment_stream_a(packet: &[u8], urb_max: usize) -> Vec<Vec<u8>> {
    let urb_max = urb_max.max(16);
    let budget_final = urb_max.saturating_sub(FRAME_OVERHEAD);
    let budget_frag = urb_max.saturating_sub(8);

    if packet.len() <= budget_final {
        return vec![pack_frame(STREAM_A_FINAL, 0, packet)];
    }
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < packet.len() {
        let remaining = packet.len() - offset;
        if remaining <= budget_final {
            out.push(pack_frame(STREAM_A_FINAL, offset as u32, &packet[offset..]));
            return out;
        }
        // Reserve at least one byte for the trailing FINAL URB (the commit marker
        // is mandatory), otherwise a packet in (budget_final, budget_frag] overruns.
        let chunk = budget_frag.min(remaining.saturating_sub(1));
        out.push(pack_fragment(
            STREAM_A_FRAG,
            offset as u32,
            &packet[offset..offset + chunk],
        ));
        offset += chunk;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turzx::{FMT_MJPEG, STREAM_B_FINAL};

    #[test]
    fn config_packet_matches_spec() {
        let pkt = build_config_packet(1920, 480, FMT_MJPEG);
        assert_eq!(pkt.len(), 28);
        assert_eq!(
            pkt,
            vec![
                0xAF, 0x20, 0x00, 0x01, 0xAF, 0x20, 0x01, 0x07, 0xAF, 0x20, 0x02, 0x80, 0xAF, 0x20,
                0x03, 0x01, 0xAF, 0x20, 0x04, 0xE0, 0xAF, 0x20, 0x11, 0x01, 0xAF, 0x20, 0x1F, 0x01,
            ]
        );
    }

    #[test]
    fn pack_frame_has_commit() {
        let pkt = pack_frame(STREAM_B_FINAL, 0, &[0xAA, 0xBB]);
        assert_eq!(
            pkt,
            vec![0xAF, 0x69, 0, 0, 0, 0, 0, 2, 0xAA, 0xBB, 0xAF, 0x66]
        );
    }

    #[test]
    fn pack_fragment_has_no_commit() {
        let pkt = pack_fragment(STREAM_A_FRAG, 0x123, &[0xCC]);
        assert_eq!(pkt, vec![0xAF, 0x6C, 0, 0x01, 0x23, 0, 0, 1, 0xCC]);
    }

    #[test]
    fn stream_a_single_urb_when_under_budget() {
        let urbs = fragment_stream_a(&[1, 2, 3, 4], 128);
        assert_eq!(urbs.len(), 1);
        assert_eq!(urbs[0][0..2], [0xAF, STREAM_A_FINAL]);
        let last_two = &urbs[0][urbs[0].len() - 2..];
        assert_eq!(last_two, &[MAGIC, COMMIT]);
    }

    #[test]
    fn stream_a_multi_urb_spans_fragments_and_ends_with_final() {
        let payload: Vec<u8> = (0..50).collect();
        let urbs = fragment_stream_a(&payload, 32);
        assert!(urbs.len() >= 2);
        for urb in urbs.iter().take(urbs.len() - 1) {
            assert_eq!(urb[1], STREAM_A_FRAG);
            let last_two = &urb[urb.len() - 2..];
            assert_ne!(last_two, &[MAGIC, COMMIT]);
        }
        let last = urbs.last().unwrap();
        assert_eq!(last[1], STREAM_A_FINAL);
        let last_two = &last[last.len() - 2..];
        assert_eq!(last_two, &[MAGIC, COMMIT]);
        let mut reassembled = Vec::new();
        for urb in &urbs {
            let size = u32::from_be_bytes([0, urb[5], urb[6], urb[7]]) as usize;
            reassembled.extend_from_slice(&urb[8..8 + size]);
        }
        assert_eq!(reassembled, payload);
    }

    #[test]
    fn stream_a_boundary_between_final_and_frag_budget_does_not_panic() {
        for len in [32758usize, 32759, 32760, 32761, 32762] {
            let payload: Vec<u8> = (0..len).map(|i| (i & 0xFF) as u8).collect();
            let urbs = fragment_stream_a(&payload, 32768);
            let mut reassembled = Vec::new();
            for urb in &urbs {
                let size = u32::from_be_bytes([0, urb[5], urb[6], urb[7]]) as usize;
                reassembled.extend_from_slice(&urb[8..8 + size]);
            }
            assert_eq!(reassembled, payload, "len={len}");
            let last = urbs.last().unwrap();
            assert_eq!(last[1], STREAM_A_FINAL, "len={len}: last urb must be FINAL");
            let last_two = &last[last.len() - 2..];
            assert_eq!(
                last_two,
                &[MAGIC, COMMIT],
                "len={len}: commit marker missing"
            );
        }
    }

    #[test]
    fn stream_a_offset_advances() {
        let payload: Vec<u8> = (0..30).collect();
        let urbs = fragment_stream_a(&payload, 20);
        let mut expected_offset = 0u32;
        for urb in &urbs {
            let off = u32::from_be_bytes([0, urb[2], urb[3], urb[4]]);
            assert_eq!(off, expected_offset);
            let size = u32::from_be_bytes([0, urb[5], urb[6], urb[7]]);
            expected_offset += size;
        }
        assert_eq!(expected_offset as usize, payload.len());
    }
}

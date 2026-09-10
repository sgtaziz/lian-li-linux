use super::engine::{frame, scale, Color, Frame, Geometry};

const L116: [&str; 29] = [
    "1d1d1d000001ffff990000ff000000000001",
    "1d1d1d000003ffff660000ff000000000001",
    "1d1d1d000005ffcc330000ff000000000001",
    "1d1d00000107ff990000ffff000000000101",
    "1d1d00000309ff660000ffff000000000101",
    "1d1d0000050bcc330000ffff000000000101",
    "1d000001070d990000ffffff000000010101",
    "1d000003090f660000ffffff000000010101",
    "1d0000050b11330000ffffff000000010101",
    "000001070d130000ffffffff000001010101",
    "000003090f150000ffffffff000001010101",
    "0000050b11170000ffffffff000001010101",
    "0001070d131900ffffffffff000101010101",
    "0003090f151b00ffffffffff000101010101",
    "00050b11171d00ffffffffff000101010101",
    "01070d13191dffffffffffff010101010101",
    "03090f151b1dffffffffffff010101010101",
    "050b11171d1dffffffffffff010101010101",
    "070d13191d1dffffffffffff010101010101",
    "090f151b1d1dffffffffffcc010101010101",
    "0b11171d1d1dffffffffff99010101010101",
    "0d13191d1d1dffffffffff66010101010101",
    "0f151b1d1d1dffffffffcc33010101010101",
    "11171d1d1d00ffffffff9900010101010101",
    "13191d1d1d00ffffffff6600010101010101",
    "151b1d1d1d00ffffffcc3300010101010101",
    "171d1d1d0000ffffff990000010101010101",
    "191d1d1d0000ffffff660000010101010101",
    "1b1d1d1d0000ffffcc330000010101010101",
];
const L88: [&str; 22] = [
    "161616000002ffff990000ff000000000001",
    "161616000004ffff660000ff000000000001",
    "161616000206ffcc3300ffff000000000101",
    "161600000408ff990000ffff000000000101",
    "16160002060aff6600ffffff000000010101",
    "16160004080ccc3300ffffff000000010101",
    "160002060a0e9900ffffffff000001010101",
    "160004080c106600ffffffff000001010101",
    "1602060a0e1233ffffffffff000101010101",
    "0004080c101400ffffffffff000101010101",
    "02060a0e1216ffffffffffff010101010101",
    "04080c101416ffffffffffff010101010101",
    "060a0e121616ffffffffffcc010101010101",
    "080c10141616ffffffffff99010101010101",
    "0a0e12161616ffffffffff66010101010101",
    "0c1014161616ffffffffcc33010101010101",
    "0e1216161616ffffffff9900010101010101",
    "101416161616ffffffff6600010101010101",
    "121616161616ffffffcc3300010101010101",
    "141616161616ffffff990000010101010101",
    "161616161616ffffff660000010101010101",
    "161616161616ffffcc330000010101010101",
];
const L132: [&str; 22] = [
    "161616000002ffff990000ff000000000001",
    "161616000004ffff660000ff000000000001",
    "161616000206ffcc3300ffff000000000101",
    "161600000408ff990000ffff000000000101",
    "16160002060aff6600ffffff000000010101",
    "16160004080ccc3300ffffff000000010101",
    "160002060a0e9900ffffffff000001010101",
    "160004080c106600ffffffff000001010101",
    "1602060a0e1233ffffffffff000101010101",
    "0004080c101400ffffffffff000101010101",
    "02060a0e1216ffffffffffff010101010101",
    "04080c101416ffffffffffff010101010101",
    "060a0e121616ffffffffffcc010101010101",
    "080c10141616ffffffffff99010101010101",
    "0a0e12161616ffffffffff66010101010101",
    "0c1014161616ffffffffcc33010101010101",
    "0e1216161616ffffffff9900010101010101",
    "101416161616ffffffff6600010101010101",
    "121616161616ffffffcc3300010101010101",
    "141616161616ffffff990000010101010101",
    "161616161616ffffff660000010101010101",
    "161616161616ffffcc330000010101010101",
];
const L174: [&str; 29] = [
    "1d1d1d000001ffff990000ff000000000001",
    "1d1d1d000003ffff660000ff000000000001",
    "1d1d1d000005ffcc330000ff000000000001",
    "1d1d00000107ff990000ffff000000000101",
    "1d1d00000309ff660000ffff000000000101",
    "1d1d0000050bcc330000ffff000000000101",
    "1d000001070d990000ffffff000000010101",
    "1d000003090f660000ffffff000000010101",
    "1d0000050b11330000ffffff000000010101",
    "000001070d130000ffffffff000001010101",
    "000003090f150000ffffffff000001010101",
    "0000050b11170000ffffffff000001010101",
    "0001070d131900ffffffffff000101010101",
    "0003090f151b00ffffffffff000101010101",
    "00050b11171d00ffffffffff000101010101",
    "01070d13191dffffffffffff010101010101",
    "03090f151b1dffffffffffff010101010101",
    "050b11171d1dffffffffffff010101010101",
    "070d13191d1dffffffffffff010101010101",
    "090f151b1d1dffffffffffcc010101010101",
    "0b11171d1d1dffffffffff99010101010101",
    "0d13191d1d1dffffffffff66010101010101",
    "0f151b1d1d1dffffffffcc33010101010101",
    "11171d1d1d00ffffffff9900010101010101",
    "13191d1d1d00ffffffff6600010101010101",
    "151b1d1d1d00ffffffcc3300010101010101",
    "171d1d1d0000ffffff990000010101010101",
    "191d1d1d0000ffffff660000010101010101",
    "1b1d1d1d0000ffffcc330000010101010101",
];

pub(super) fn shock_wave(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let states: &[&str] = match geometry.led_count {
        88 => &L88,
        116 => &L116,
        132 => &L132,
        174 => &L174,
        _ => unreachable!(),
    };
    let mut output = Vec::with_capacity(states.len() * 6);
    for base_color in 0..6 {
        for state in states {
            let values = (0..18)
                .map(|index| {
                    let offset = index * 2;
                    (hex(state.as_bytes()[offset]) << 4) | hex(state.as_bytes()[offset + 1])
                })
                .collect::<Vec<_>>();
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                let size = usize::from(values[lane]);
                let intensity = values[6 + lane];
                let color = (base_color + usize::from(values[12 + lane])) % 6;
                let start = (geometry.lane_length - size) / 2;
                current[lane * geometry.lane_length + start
                    ..lane * geometry.lane_length + start + size]
                    .fill(scale(scale(colors[color], intensity), brightness));
            }
            if reverse {
                current.reverse();
            }
            output.push(current);
        }
    }
    output
}

fn hex(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!(),
    }
}

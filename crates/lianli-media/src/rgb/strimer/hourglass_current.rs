use super::engine::{frame, scale, Color, Frame, Geometry};

const HOURGLASS_22: [&str; 43] = [
    "ff0000000000000000000000000000000000000000ff",
    "ffff000000000000000000000000000000000000ffff",
    "ffffff00000000000000000000000000000000ffffff",
    "ffffffff0000000000000000000000000000ffffffff",
    "ffffffffff000000000000000000000000ffffffffff",
    "ffffffffffff00000000000000000000ffffffffffff",
    "ffffffffffffff0000000000000000ffffffffffffff",
    "ffffffffffffffff000000000000ffffffffffffffff",
    "ffffffffffffffffff00000000ffffffffffffffffff",
    "ffffffffffffffffffff0000ffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffff",
    "ff000000000000000000ffff000000000000000000ff",
    "ffff0000000000000000ffff0000000000000000ffff",
    "ffffff00000000000000ffff00000000000000ffffff",
    "ffffffff000000000000ffff000000000000ffffffff",
    "ffffffffff0000000000ffff0000000000ffffffffff",
    "ffffffffffff00000000ffff00000000ffffffffffff",
    "ffffffffffffff000000ffff000000ffffffffffffff",
    "ffffffffffffffff0000ffff0000ffffffffffffffff",
    "ffffffffffffffffff00ffff00ffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffff",
    "ff0000000000000000ffffffff0000000000000000ff",
    "ffff00000000000000ffffffff00000000000000ffff",
    "ffffff000000000000ffffffff000000000000ffffff",
    "ffffffff0000000000ffffffff0000000000ffffffff",
    "ffffffffff00000000ffffffff00000000ffffffffff",
    "ffffffffffff000000ffffffff000000ffffffffffff",
    "ffffffffffffff0000ffffffff0000ffffffffffffff",
    "ffffffffffffffff00ffffffff00ffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffff",
    "0000000000000000e6e6e6e6e6e60000000000000000",
    "00000000000000cdcdcdcdcdcdcdcd00000000000000",
    "000000000000b4b4b4b4b4b4b4b4b4b4000000000000",
    "00000000009b9b9b9b9b9b9b9b9b9b9b9b0000000000",
    "00000000828282828282828282828282828200000000",
    "00000069696969696969696969696969696969000000",
    "00005050505050505050505050505050505050500000",
    "00373737373737373737373737373737373737373700",
    "1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e",
    "14141414141414141414141414141414141414141414",
    "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a",
    "00000000000000000000000000000000000000000000",
];
const HOURGLASS_29: [&str; 66] = [
    "ff000000000000000000000000000000000000000000000000000000ff",
    "ffff00000000000000000000000000000000000000000000000000ffff",
    "ffffff0000000000000000000000000000000000000000000000ffffff",
    "ffffffff000000000000000000000000000000000000000000ffffffff",
    "ffffffffff00000000000000000000000000000000000000ffffffffff",
    "ffffffffffff0000000000000000000000000000000000ffffffffffff",
    "ffffffffffffff000000000000000000000000000000ffffffffffffff",
    "ffffffffffffffff00000000000000000000000000ffffffffffffffff",
    "ffffffffffffffffff0000000000000000000000ffffffffffffffffff",
    "ffffffffffffffffffff000000000000000000ffffffffffffffffffff",
    "ffffffffffffffffffffff00000000000000ffffffffffffffffffffff",
    "ffffffffffffffffffffffff0000000000ffffffffffffffffffffffff",
    "ffffffffffffffffffffffffff000000ffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffff00ffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ff00000000000000000000000000ff00000000000000000000000000ff",
    "ffff000000000000000000000000ff000000000000000000000000ffff",
    "ffffff0000000000000000000000ff0000000000000000000000ffffff",
    "ffffffff00000000000000000000ff00000000000000000000ffffffff",
    "ffffffffff000000000000000000ff000000000000000000ffffffffff",
    "ffffffffffff0000000000000000ff0000000000000000ffffffffffff",
    "ffffffffffffff00000000000000ff00000000000000ffffffffffffff",
    "ffffffffffffffff000000000000ff000000000000ffffffffffffffff",
    "ffffffffffffffffff0000000000ff0000000000ffffffffffffffffff",
    "ffffffffffffffffffff00000000ff00000000ffffffffffffffffffff",
    "ffffffffffffffffffffff000000ff000000ffffffffffffffffffffff",
    "ffffffffffffffffffffffff0000ff0000ffffffffffffffffffffffff",
    "ffffffffffffffffffffffffff00ff00ffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ff000000000000000000000000ffffff000000000000000000000000ff",
    "ffff0000000000000000000000ffffff0000000000000000000000ffff",
    "ffffff00000000000000000000ffffff00000000000000000000ffffff",
    "ffffffff000000000000000000ffffff000000000000000000ffffffff",
    "ffffffffff0000000000000000ffffff0000000000000000ffffffffff",
    "ffffffffffff00000000000000ffffff00000000000000ffffffffffff",
    "ffffffffffffff000000000000ffffff000000000000ffffffffffffff",
    "ffffffffffffffff0000000000ffffff0000000000ffffffffffffffff",
    "ffffffffffffffffff00000000ffffff00000000ffffffffffffffffff",
    "ffffffffffffffffffff000000ffffff000000ffffffffffffffffffff",
    "ffffffffffffffffffffff0000ffffff0000ffffffffffffffffffffff",
    "ffffffffffffffffffffffff00ffffff00ffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ff0000000000000000000000ffffffffff0000000000000000000000ff",
    "ffff00000000000000000000ffffffffff00000000000000000000ffff",
    "ffffff000000000000000000ffffffffff000000000000000000ffffff",
    "ffffffff0000000000000000ffffffffff0000000000000000ffffffff",
    "ffffffffff00000000000000ffffffffff00000000000000ffffffffff",
    "ffffffffffff000000000000ffffffffff000000000000ffffffffffff",
    "ffffffffffffff0000000000ffffffffff0000000000ffffffffffffff",
    "ffffffffffffffff00000000ffffffffff00000000ffffffffffffffff",
    "ffffffffffffffffff000000ffffffffff000000ffffffffffffffffff",
    "ffffffffffffffffffff0000ffffffffff0000ffffffffffffffffffff",
    "ffffffffffffffffffffff00ffffffffff00ffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "0000000000000000000000e6e6e6e6e6e6e60000000000000000000000",
    "00000000000000000000cdcdcdcdcdcdcdcdcd00000000000000000000",
    "000000000000000000b4b4b4b4b4b4b4b4b4b4b4000000000000000000",
    "00000000000000009b9b9b9b9b9b9b9b9b9b9b9b9b0000000000000000",
    "0000000000000082828282828282828282828282828200000000000000",
    "0000000000006969696969696969696969696969696969000000000000",
    "0000000000505050505050505050505050505050505050500000000000",
    "0000000037373737373737373737373737373737373737373700000000",
    "0000001e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e000000",
    "0000141414141414141414141414141414141414141414141414140000",
    "000a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a00",
    "0000000000000000000000000000000000000000000000000000000000",
];
const CURRENT_22: [&str; 40] = [
    "0000000000000000000000",
    "1000000000000000000001",
    "1100000000000000000011",
    "1110000000000000000111",
    "1111000000000000001111",
    "1111100000000000011111",
    "2111110000000000111112",
    "2211111000000001111122",
    "2221111100000011111222",
    "2222111110000111112222",
    "2222211111001111122222",
    "0222221111111111222220",
    "0022222111111112222200",
    "0002222211111122222000",
    "0000222221111222220000",
    "0000022222112222200000",
    "0000002222222222000000",
    "0000000222222220000000",
    "0000000022222200000000",
    "0000000002222000000000",
    "0000000000220000000000",
    "0000000002222000000000",
    "0000000022222200000000",
    "0000000222222220000000",
    "0000002222222222000000",
    "0000022222112222200000",
    "0000222221111222220000",
    "0002222211111222222000",
    "0022222111111112222200",
    "0222221111111111222220",
    "2222211111001111122222",
    "2222111110000111112222",
    "2221111100000111111222",
    "2211111000000001111122",
    "2111110000000000111112",
    "1111100000000000011111",
    "1111000000000000001111",
    "1110000000000000000111",
    "1100000000000000000011",
    "1000000000000000000001",
];
const CURRENT_29: [&str; 51] = [
    "00000000000000000000000000000",
    "10000000000000000000000000001",
    "11000000000000000000000000011",
    "11100000000000000000000000111",
    "11110000000000000000000001111",
    "11111000000000000000000011111",
    "11111100000000000000000111111",
    "21111110000000000000001111112",
    "22111111000000000000011111122",
    "22211111100000000000111111222",
    "22221111110000000001111112222",
    "22222111111000000011111122222",
    "22222211111100000111111222222",
    "02222221111110001111112222220",
    "00222222111111011111122222200",
    "00022222211111111111222222000",
    "00002222221111111112222220000",
    "00000222222111111122222200000",
    "00000022222211111222222000000",
    "00000002222221112222220000000",
    "00000000222222122222200000000",
    "00000000022222222222000000000",
    "00000000002222222220000000000",
    "00000000000222222200000000000",
    "00000000000022222000000000000",
    "00000000000002220000000000000",
    "00000000000000200000000000000",
    "00000000000002220000000000000",
    "00000000000022222000000000000",
    "00000000000222222200000000000",
    "00000000002222222220000000000",
    "00000000022222122222000000000",
    "00000000222221112222200000000",
    "00000002222211111222220000000",
    "00000022222111111122222000000",
    "00000222221111111112222200000",
    "00002222211111111111222220000",
    "00022222111111011111122222000",
    "00222221111110001111112222200",
    "02222211111100000111111222220",
    "22222111111000000011111122222",
    "22221111110000000001111112222",
    "22211111100000000000111111222",
    "22111111000000000000011111122",
    "21111110000000000000001111112",
    "11111100000000000000000111111",
    "11111000000000000000000011111",
    "11110000000000000000000001111",
    "11100000000000000000000000111",
    "11000000000000000000000000011",
    "10000000000000000000000000001",
];

pub(super) fn hourglass(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let table: &[&str] = if geometry.lane_length == 22 {
        &HOURGLASS_22
    } else {
        &HOURGLASS_29
    };
    let mut output = Vec::with_capacity(table.len() * 4);
    for color in colors.iter().take(4) {
        for row in table {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..geometry.lane_length {
                    let offset = position * 2;
                    let intensity =
                        (hex(row.as_bytes()[offset]) << 4) | hex(row.as_bytes()[offset + 1]);
                    current[lane * geometry.lane_length + position] =
                        scale(scale(*color, intensity), brightness);
                }
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

pub(super) fn electric_current(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
) -> Vec<Frame> {
    let (table, threshold): (&[&str], usize) = if geometry.lane_length == 22 {
        (&CURRENT_22, 20)
    } else {
        (&CURRENT_29, 27)
    };
    let rows = std::iter::repeat_n(0, 5)
        .chain(1..=threshold)
        .chain(((threshold + 2)..table.len()).step_by(2))
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(rows.len() * 4);
    for color in colors.iter().take(4) {
        for &row in &rows {
            let mut current = frame(geometry);
            for lane in 0..geometry.lanes {
                for (position, token) in table[row].bytes().enumerate() {
                    let pixel = match token {
                        b'0' => [0; 3],
                        b'2' if lane == 0 || lane + 1 == geometry.lanes => [255; 3],
                        _ => *color,
                    };
                    current[lane * geometry.lane_length + position] = scale(pixel, brightness);
                }
            }
            output.push(current);
        }
    }
    output
}

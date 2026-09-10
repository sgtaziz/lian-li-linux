use lianli_shared::rgb::RgbEffect;

pub(super) type Color = [u8; 3];

pub(super) fn palette(effect: &RgbEffect) -> [Color; 6] {
    // Older configurations allowed incomplete palettes; missing slots are not intentional black.
    let mut colors = [effect.colors.last().copied().unwrap_or([0; 3]); 6];
    for (target, source) in colors.iter_mut().zip(&effect.colors) {
        *target = *source;
    }
    colors
}

pub(super) fn scale(color: Color, value: u8) -> Color {
    color.map(|channel| ((u16::from(channel) * u16::from(value)) >> 8) as u8)
}

pub(super) fn frame(led_count: usize) -> Vec<Color> {
    vec![[0; 3]; led_count]
}

pub(super) fn map_31(source: &[Color; 31], led_count: usize) -> Vec<Color> {
    let mut output = frame(led_count);
    output[47..60].copy_from_slice(&source[..13]);
    output[..18].copy_from_slice(&source[13..31]);
    for index in 0..31 {
        output[47 - index] = source[index];
    }
    output
}

pub(super) fn map_16(source: &[Color; 16], led_count: usize) -> Vec<Color> {
    let mut output = frame(led_count);
    output[47..60].copy_from_slice(&source[..13]);
    output[..3].copy_from_slice(&source[13..16]);
    for index in 0..16 {
        output[17 - index] = source[index];
        output[47 - index] = source[index];
        output[index + 17] = source[index];
    }
    output
}

pub(super) fn map_32(source: &[Color; 32], led_count: usize) -> Vec<Color> {
    let mut output = frame(led_count);
    output[47..60].copy_from_slice(&source[..13]);
    output[..3].copy_from_slice(&source[13..16]);
    for index in 0..16 {
        output[17 - index] = source[31 - index];
        output[47 - index] = source[index];
        output[index + 17] = source[31 - index];
    }
    output
}

pub(super) fn map_25(source: &[Color; 25], led_count: usize) -> Vec<Color> {
    let mut output = frame(led_count);
    output[44..51].fill(source[0]);
    output[51..60].copy_from_slice(&source[1..10]);
    output[..14].copy_from_slice(&source[10..24]);
    output[14..21].fill(source[24]);
    for index in 0..23 {
        output[43 - index] = source[index + 1];
    }
    output
}

pub(super) struct DotNetRandom {
    seeds: [i32; 56],
    next: usize,
    peer: usize,
}

impl DotNetRandom {
    pub(super) fn new(seed: i32) -> Self {
        const BIG: i32 = i32::MAX;
        let subtraction = if seed == i32::MIN { BIG } else { seed.abs() };
        let mut seeds = [0; 56];
        let mut current = 161_803_398 - subtraction;
        if current < 0 {
            current += BIG;
        }
        seeds[55] = current;
        let mut previous = 1;
        for index in 1..55 {
            let destination = 21 * index % 55;
            seeds[destination] = previous;
            previous = current - previous;
            if previous < 0 {
                previous += BIG;
            }
            current = seeds[destination];
        }
        for _ in 0..4 {
            for index in 1..56 {
                seeds[index] -= seeds[1 + (index + 30) % 55];
                if seeds[index] < 0 {
                    seeds[index] += BIG;
                }
            }
        }
        Self {
            seeds,
            next: 0,
            peer: 21,
        }
    }

    pub(super) fn next(&mut self, max: i32) -> i32 {
        const BIG: i32 = i32::MAX;
        self.next += 1;
        if self.next >= 56 {
            self.next = 1;
        }
        self.peer += 1;
        if self.peer >= 56 {
            self.peer = 1;
        }
        let mut sample = self.seeds[self.next] - self.seeds[self.peer];
        if sample == BIG {
            sample -= 1;
        }
        if sample < 0 {
            sample += BIG;
        }
        self.seeds[self.next] = sample;
        (f64::from(sample) * (1.0 / f64::from(BIG)) * f64::from(max)) as i32
    }
}

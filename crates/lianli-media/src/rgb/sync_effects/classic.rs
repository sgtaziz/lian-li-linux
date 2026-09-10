use super::engine::{scale, solid_frame, Color, Frames};

const RAINBOW: [Color; 24] = [
    [255, 0, 0],
    [224, 32, 0],
    [192, 64, 0],
    [160, 96, 0],
    [128, 128, 0],
    [96, 160, 0],
    [64, 192, 0],
    [32, 224, 0],
    [0, 255, 0],
    [0, 224, 32],
    [0, 192, 64],
    [0, 160, 96],
    [0, 128, 128],
    [0, 96, 160],
    [0, 64, 192],
    [0, 32, 224],
    [0, 0, 255],
    [32, 0, 224],
    [64, 0, 192],
    [96, 0, 160],
    [128, 0, 128],
    [160, 0, 96],
    [192, 0, 64],
    [224, 0, 32],
];

pub(super) fn rainbow(leds: usize, bright: u16, reverse: bool) -> Frames {
    (0..24)
        .map(|shift| {
            (0..leds)
                .map(|position| {
                    let destination = position % 24;
                    let source = if reverse {
                        destination
                    } else {
                        23 - destination
                    };
                    scale(RAINBOW[(shift + source) % 24], bright)
                })
                .collect()
        })
        .collect()
}

pub(super) fn static_color(leds: usize, color: Color, bright: u16) -> Frames {
    vec![solid_frame(leds, scale(color, bright))]
}

pub(super) fn breathing(leds: usize, color: Color, bright: u16) -> Frames {
    let mut level = 0usize;
    (0..170)
        .map(|frame| {
            let intensity = (level * 3) as u16;
            let output = solid_frame(leds, scale(scale(color, intensity), bright));
            if frame >= 85 {
                level -= 1;
            } else {
                level += 1;
            }
            output
        })
        .collect()
}

pub(super) fn runway(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    let width = 12;
    let mut frames = Vec::with_capacity(2 * (leds + width - 1));
    for reversed_pass in [false, true] {
        for step in 0..leds + width - 1 {
            let mut frame = vec![[0; 3]; leds];
            for position in 0..leds {
                let source = usize::from(!(position <= step && position + width > step));
                let destination = if reversed_pass {
                    leds - position - 1
                } else {
                    position
                };
                frame[destination] = scale(colors[source], bright);
            }
            frames.push(frame);
        }
    }
    frames
}

pub(super) fn meteor(leds: usize, color: Color, bright: u16, reverse: bool) -> Frames {
    const INTENSITY: [u16; 12] = [12, 16, 20, 24, 32, 40, 48, 64, 96, 128, 192, 255];
    let width = INTENSITY.len();
    let mut ribbon = vec![[0; 3]; leds + width];
    for (target, intensity) in ribbon.iter_mut().zip(INTENSITY) {
        *target = scale(color, intensity);
    }
    let mut offset = width;
    let mut frames = Vec::with_capacity(ribbon.len());
    for _ in 0..ribbon.len() {
        let mut frame = vec![[0; 3]; leds];
        for position in 0..leds {
            let source = (offset + position) % ribbon.len();
            let destination = if reverse {
                leds - position - 1
            } else {
                position
            };
            frame[destination] = scale(ribbon[source], bright);
        }
        frames.push(frame);
        offset = if offset == 0 {
            ribbon.len() - 1
        } else {
            offset - 1
        };
    }
    frames
}

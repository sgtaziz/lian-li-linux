use super::geometry::{frame, Color};
use lianli_shared::rgb::RgbEffect;
use std::f64::consts::TAU;

const RING: usize = 60;
const RED: Color = [255, 0, 40];
const BLUE: Color = [0, 60, 255];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n - 1)
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

fn pair(effect: &RgbEffect) -> (Color, Color) {
    match effect.colors.as_slice() {
        [] => (RED, BLUE),
        [one] => (*one, shade(*one, 0.35)),
        [first, second, ..] => (*first, *second),
    }
}

fn shade(color: Color, amount: f64) -> Color {
    color.map(|c| (f64::from(c) * amount).round().clamp(0.0, 255.0) as u8)
}

fn mix(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (f64::from(a[i]) + (f64::from(b[i]) - f64::from(a[i])) * t).round() as u8)
}

fn output(led_count: usize, ring: &[Color], brightness: u8, reverse: bool) -> Vec<Color> {
    let mut out = frame(led_count);
    for (index, color) in ring.iter().enumerate() {
        let target = if reverse {
            (RING - index) % RING
        } else {
            index
        };
        out[target] = shade(*color, f64::from(brightness) / 255.0);
    }
    out
}

fn ring_distance(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(RING as f64);
    d.min(RING as f64 - d)
}

pub(super) fn nebula(
    effect: &RgbEffect,
    led_count: usize,
    brightness: u8,
    reverse: bool,
) -> Vec<Vec<Color>> {
    const LENGTH: usize = 480;
    let (first, second) = pair(effect);
    let mut rng = Rng(0x6E65_6275_6C61);
    let waves: Vec<(f64, f64, f64)> = (0..3)
        .map(|_| {
            (
                rng.pick(&[1.0, 2.0, 3.0]),
                rng.pick(&[-2.0, -1.0, 1.0, 2.0]),
                rng.range(0.0, TAU),
            )
        })
        .collect();
    let glows: Vec<(f64, f64, f64)> = (0..2)
        .map(|_| {
            (
                rng.pick(&[2.0, 3.0, 4.0]),
                rng.pick(&[-3.0, -2.0, 2.0, 3.0]),
                rng.range(0.0, TAU),
            )
        })
        .collect();
    (0..LENGTH)
        .map(|f| {
            let time = f as f64 / LENGTH as f64 * TAU;
            let ring: Vec<Color> = (0..RING)
                .map(|i| {
                    let x = i as f64 / RING as f64 * TAU;
                    let sum: f64 = waves
                        .iter()
                        .map(|(k, w, p)| (k * x + w * time + p).sin())
                        .sum();
                    let blend = sum / 3.0 * 0.5 + 0.5;
                    let blend = blend.powi(3) / (blend.powi(3) + (1.0 - blend).powi(3));
                    let lit: f64 = glows
                        .iter()
                        .map(|(k, w, p)| (k * x + w * time + p).sin())
                        .sum();
                    let glow = (lit / 2.0 * 0.5 + 0.5).powf(2.4);
                    shade(mix(first, second, blend), 0.03 + 0.97 * glow)
                })
                .collect();
            output(led_count, &ring, brightness, reverse)
        })
        .collect()
}

pub(super) fn cinder(effect: &RgbEffect, led_count: usize, brightness: u8) -> Vec<Vec<Color>> {
    const LENGTH: usize = 600;
    let (hot, cool) = pair(effect);
    let mut rng = Rng(0x6369_6E64_6572);
    let mut flares: Vec<Vec<(usize, f64)>> = vec![Vec::new(); LENGTH];
    for _ in 0..LENGTH * 9 / 10 {
        let at = rng.below(LENGTH);
        flares[at].push((rng.below(RING), rng.range(0.6, 1.0)));
    }
    let mut heat = [0.0f64; RING];
    let mut frames = Vec::with_capacity(LENGTH);
    for step in 0..LENGTH * 2 {
        for value in heat.iter_mut() {
            *value *= 0.86;
        }
        for &(led, power) in &flares[step % LENGTH] {
            heat[led] = heat[led].max(power);
            let right = (led + 1) % RING;
            let left = (led + RING - 1) % RING;
            heat[right] = heat[right].max(power * 0.5);
            heat[left] = heat[left].max(power * 0.5);
        }
        if step >= LENGTH {
            let ring: Vec<Color> = heat
                .iter()
                .map(|&h| shade(mix(cool, hot, h * 1.4), h.powf(1.3)))
                .collect();
            frames.push(output(led_count, &ring, brightness, false));
        }
    }
    frames
}

struct Lap {
    origin: f64,
    split: f64,
    length: f64,
}

pub(super) fn pulsar(effect: &RgbEffect, led_count: usize, brightness: u8) -> Vec<Vec<Color>> {
    const LAPS: usize = 12;
    const STEP: f64 = 2.2;
    const LAG: f64 = 3.0;
    let (lead, echo) = pair(effect);
    let ring = RING as f64;
    let mut rng = Rng(0x7075_6C73_6172);
    let mut laps = Vec::with_capacity(LAPS);
    let mut origin = 0.0;
    let mut push = |rng: &mut Rng, origin: &mut f64, split: f64| {
        let length = ring / (STEP * rng.range(0.8, 1.25));
        laps.push(Lap {
            origin: *origin,
            split,
            length,
        });
        *origin = (*origin + split * ring).rem_euclid(ring);
    };
    for _ in 0..LAPS - 2 {
        let split = rng.range(0.3, 0.7);
        push(&mut rng, &mut origin, split);
    }
    let (closing, last) = loop {
        let first = rng.range(0.3, 0.7);
        let rest = (-(origin + first * ring)).rem_euclid(ring) / ring;
        if (0.3..=0.7).contains(&rest) {
            break (first, rest);
        }
    };
    push(&mut rng, &mut origin, closing);
    push(&mut rng, &mut origin, last);

    let natural: f64 = laps.iter().map(|lap| lap.length).sum();
    let total = natural.round().max(1.0);
    let stretch = total / natural;
    let mut starts = Vec::with_capacity(laps.len());
    let mut clock = 0.0;
    for lap in &laps {
        starts.push(clock);
        clock += lap.length * stretch;
    }
    let fronts = |time: f64| {
        let time = time.rem_euclid(total);
        let index = starts.iter().rposition(|&start| start <= time).unwrap_or(0);
        let lap = &laps[index];
        let progress = (time - starts[index]) / (lap.length * stretch);
        (
            (lap.origin + progress * lap.split * ring).rem_euclid(ring),
            (lap.origin - progress * (1.0 - lap.split) * ring).rem_euclid(ring),
        )
    };
    let glow = |at: f64, a: f64, b: f64| {
        (-ring_distance(at, a).powi(2) / 5.0)
            .exp()
            .max((-ring_distance(at, b).powi(2) / 5.0).exp())
    };
    (0..total as usize)
        .map(|f| {
            let (a, b) = fronts(f as f64);
            let (trail_a, trail_b) = fronts(f as f64 - LAG);
            let ring: Vec<Color> = (0..RING)
                .map(|i| {
                    let main = glow(i as f64, a, b);
                    let trail = 0.55 * glow(i as f64, trail_a, trail_b);
                    if main >= trail {
                        shade(lead, main)
                    } else {
                        shade(echo, trail)
                    }
                })
                .collect();
            output(led_count, &ring, brightness, false)
        })
        .collect()
}

pub(super) fn steady_nebula(
    effect: &RgbEffect,
    led_count: usize,
    brightness: u8,
    reverse: bool,
) -> Vec<Vec<Color>> {
    const LENGTH: usize = 240;
    let (first, second) = pair(effect);
    (0..LENGTH)
        .map(|f| {
            let time = f as f64 / LENGTH as f64 * TAU;
            let ring: Vec<Color> = (0..RING)
                .map(|i| {
                    let x = i as f64 / RING as f64 * TAU;
                    let blend = 0.5 + 0.5 * (2.0 * x + time).sin();
                    let blend = blend.powi(3) / (blend.powi(3) + (1.0 - blend).powi(3));
                    let glow = (0.5 + 0.5 * (3.0 * x - 2.0 * time).sin()).powf(2.2);
                    shade(mix(first, second, blend), 0.03 + 0.97 * glow)
                })
                .collect();
            output(led_count, &ring, brightness, reverse)
        })
        .collect()
}

pub(super) fn steady_cinder(
    effect: &RgbEffect,
    led_count: usize,
    brightness: u8,
) -> Vec<Vec<Color>> {
    const LENGTH: usize = 200;
    let (hot, cool) = pair(effect);
    (0..LENGTH)
        .map(|f| {
            let time = f as f64 / LENGTH as f64;
            let ring: Vec<Color> = (0..RING)
                .map(|i| {
                    let phase = time * 2.0 + i as f64 * 7.0 / RING as f64;
                    let h = (0.5 + 0.5 * (phase * TAU).sin()).powi(4);
                    shade(mix(cool, hot, h * 1.4), h.powf(1.3))
                })
                .collect();
            output(led_count, &ring, brightness, false)
        })
        .collect()
}

pub(super) fn steady_pulsar(
    effect: &RgbEffect,
    led_count: usize,
    brightness: u8,
) -> Vec<Vec<Color>> {
    const LENGTH: usize = 55;
    const LAG: f64 = 3.0;
    let (lead, echo) = pair(effect);
    let ring = RING as f64;
    let fronts = |time: f64| {
        let travel = time.rem_euclid(LENGTH as f64) / LENGTH as f64 * ring;
        (travel.rem_euclid(ring), (-travel).rem_euclid(ring))
    };
    let glow = |at: f64, a: f64, b: f64| {
        (-ring_distance(at, a).powi(2) / 5.0)
            .exp()
            .max((-ring_distance(at, b).powi(2) / 5.0).exp())
    };
    (0..LENGTH)
        .map(|f| {
            let (a, b) = fronts(f as f64);
            let (trail_a, trail_b) = fronts(f as f64 - LAG);
            let ring: Vec<Color> = (0..RING)
                .map(|i| {
                    let main = glow(i as f64, a, b);
                    let trail = 0.55 * glow(i as f64, trail_a, trail_b);
                    if main >= trail {
                        shade(lead, main)
                    } else {
                        shade(echo, trail)
                    }
                })
                .collect();
            output(led_count, &ring, brightness, false)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(mode: RgbMode, colors: Vec<Color>) -> RgbEffect {
        RgbEffect {
            mode,
            colors,
            ..RgbEffect::default()
        }
    }

    fn jump(a: &[Color], b: &[Color]) -> f64 {
        a.iter()
            .zip(b)
            .flat_map(|(x, y)| x.iter().zip(y))
            .map(|(p, q)| (f64::from(*p) - f64::from(*q)).abs())
            .sum::<f64>()
            / (RING * 3) as f64
    }

    fn all(colors: Vec<Color>) -> Vec<Vec<Vec<Color>>> {
        vec![
            nebula(
                &effect(RgbMode::NebulaRandom, colors.clone()),
                60,
                255,
                false,
            ),
            cinder(&effect(RgbMode::CinderRandom, colors.clone()), 60, 255),
            pulsar(&effect(RgbMode::PulsarRandom, colors.clone()), 60, 255),
            steady_nebula(&effect(RgbMode::Nebula, colors.clone()), 60, 255, false),
            steady_cinder(&effect(RgbMode::Cinder, colors.clone()), 60, 255),
            steady_pulsar(&effect(RgbMode::Pulsar, colors), 60, 255),
        ]
    }

    #[test]
    fn loops_join_up_without_a_visible_jump() {
        for frames in all(vec![]) {
            let typical = frames
                .windows(2)
                .map(|w| jump(&w[0], &w[1]))
                .fold(0.0, f64::max);
            let seam = jump(frames.last().unwrap(), &frames[0]);
            assert!(
                seam <= typical * 1.6 + 1.0,
                "seam {seam} vs largest step {typical}"
            );
        }
    }

    #[test]
    fn frames_fit_the_ring_and_the_frame_limit() {
        for frames in all(vec![[10, 200, 30]]) {
            assert!(
                !frames.is_empty() && frames.len() <= lianli_shared::rgb::MAX_RGB_ANIMATION_FRAMES
            );
            assert!(frames.iter().all(|f| f.len() == 60));
            assert!(frames.iter().flatten().any(|c| *c != [0; 3]));
        }
    }

    #[test]
    fn colours_come_from_the_palette() {
        for frames in all(vec![[0, 255, 0], [0, 0, 255]]) {
            assert!(frames.iter().flatten().all(|c| c[0] == 0));
        }
    }

    #[test]
    fn brightness_scales_and_zero_is_dark() {
        let bright = pulsar(&effect(RgbMode::Pulsar, vec![]), 60, 255);
        let dim = pulsar(&effect(RgbMode::Pulsar, vec![]), 60, 64);
        let peak =
            |frames: &Vec<Vec<Color>>| frames.iter().flatten().flatten().copied().max().unwrap();
        assert!(peak(&dim) < peak(&bright));
        assert!(pulsar(&effect(RgbMode::Pulsar, vec![]), 60, 0)
            .iter()
            .flatten()
            .all(|c| *c == [0; 3]));
    }

    #[test]
    fn pulsar_starts_somewhere_new_each_lap() {
        let frames = pulsar(&effect(RgbMode::PulsarRandom, vec![]), 60, 255);
        let brightest: Vec<usize> = frames
            .iter()
            .map(|f| {
                (0..60)
                    .max_by_key(|&i| f[i].iter().map(|&c| u32::from(c)).sum::<u32>())
                    .unwrap()
            })
            .collect();
        let mut seen = brightest.clone();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() > 40, "pulses only visited {} LEDs", seen.len());
    }

    #[test]
    fn every_speed_brightness_and_direction_renders() {
        use lianli_shared::rgb::RgbDirection;
        for mode in [
            RgbMode::Nebula,
            RgbMode::Cinder,
            RgbMode::Pulsar,
            RgbMode::NebulaRandom,
            RgbMode::CinderRandom,
            RgbMode::PulsarRandom,
        ] {
            for speed in 0..=4 {
                for brightness in 0..=4 {
                    for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                        let effect = RgbEffect {
                            mode,
                            speed,
                            brightness,
                            direction,
                            ..RgbEffect::default()
                        };
                        let animation = super::super::render(&effect, 60).unwrap();
                        assert!((100..=6_553_599).contains(&animation.interval_hundredths));
                        assert_eq!(
                            animation.interval_hundredths,
                            1_600 * [7, 6, 5, 4, 3][speed as usize]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn steady_pulsar_always_starts_from_the_same_spot() {
        let frames = steady_pulsar(&effect(RgbMode::Pulsar, vec![]), 60, 255);
        let peak = (0..60)
            .max_by_key(|&i| frames[0][i].iter().map(|&c| u32::from(c)).sum::<u32>())
            .unwrap();
        assert_eq!(peak, 0);
    }

    #[test]
    fn output_is_deterministic() {
        assert_eq!(all(vec![[1, 2, 3]]), all(vec![[1, 2, 3]]));
    }
}

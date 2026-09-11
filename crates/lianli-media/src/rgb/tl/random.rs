use super::engine::Color;
pub(super) const COLORS: [Color; 13] = [
    [255, 0, 0],
    [255, 128, 0],
    [255, 255, 0],
    [128, 255, 0],
    [0, 255, 0],
    [0, 255, 128],
    [0, 255, 255],
    [0, 128, 255],
    [0, 0, 255],
    [128, 0, 255],
    [255, 0, 255],
    [255, 0, 128],
    [255, 255, 255],
];

pub(super) struct DotNetRandom {
    seed_array: [i32; 56],
    next: usize,
    next_peer: usize,
}

impl DotNetRandom {
    pub(super) fn new(seed: i32) -> Self {
        const BIG: i32 = i32::MAX;
        const MAGIC: i32 = 161_803_398;

        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.abs()
        };
        let mut seed_array = [0; 56];
        let mut current = MAGIC - subtraction;
        if current < 0 {
            current += BIG;
        }
        seed_array[55] = current;
        let mut previous = 1;
        for index in 1..55 {
            let destination = 21 * index % 55;
            seed_array[destination] = previous;
            previous = current - previous;
            if previous < 0 {
                previous += BIG;
            }
            current = seed_array[destination];
        }
        for _ in 0..4 {
            for index in 1..56 {
                seed_array[index] -= seed_array[1 + (index + 30) % 55];
                if seed_array[index] < 0 {
                    seed_array[index] += BIG;
                }
            }
        }
        Self {
            seed_array,
            next: 0,
            next_peer: 21,
        }
    }

    pub(super) fn next(&mut self, min: i32, max: i32) -> i32 {
        (self.next_double() * f64::from(max - min)) as i32 + min
    }

    pub(super) fn next_double(&mut self) -> f64 {
        const BIG: i32 = i32::MAX;

        self.next += 1;
        if self.next >= 56 {
            self.next = 1;
        }
        self.next_peer += 1;
        if self.next_peer >= 56 {
            self.next_peer = 1;
        }
        let mut sample = self.seed_array[self.next] - self.seed_array[self.next_peer];
        if sample == BIG {
            sample -= 1;
        }
        if sample < 0 {
            sample += BIG;
        }
        self.seed_array[self.next] = sample;
        f64::from(sample) * (1.0 / f64::from(BIG))
    }
}

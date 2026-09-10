use super::engine::{frame, scale, Color};

pub(super) fn rainbow(brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const COLORS: [Color; 24] = [
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
    (0..24)
        .map(|shift| {
            let mut output = frame();
            for position in 0..24 {
                let destination = if reverse { position } else { 23 - position };
                output[destination] = scale(COLORS[(shift + position) % 24], brightness);
            }
            output
        })
        .collect()
}

pub(super) fn morph(brightness: u8) -> Vec<Vec<Color>> {
    let (mut red, mut green, mut blue) = (255i16, 0i16, 0i16);
    let mut source = Vec::with_capacity(255);
    for index in 0..255 {
        source.push(vec![
            scale([red as u8, green as u8, blue as u8], brightness);
            24
        ]);
        if index < 85 {
            red -= 3;
            green += 3;
            blue = 0;
        } else if index < 170 {
            red = 0;
            green -= 3;
            blue += 3;
        } else {
            red += 3;
            green = 0;
            blue -= 3;
        }
    }
    (0..127).map(|index| source[index * 2].clone()).collect()
}

pub(super) fn solid(color: Color, brightness: u8) -> Vec<Vec<Color>> {
    vec![vec![scale(color, brightness); 24]; 30]
}

pub(super) fn breathing(color: Color, brightness: u8) -> Vec<Vec<Color>> {
    let mut phase = 0i16;
    (0..170)
        .map(|index| {
            let intensity = ((phase * 3) & 0xff) as u8;
            phase += if index >= 85 { -1 } else { 1 };
            vec![scale(scale(color, intensity), brightness); 24]
        })
        .collect()
}

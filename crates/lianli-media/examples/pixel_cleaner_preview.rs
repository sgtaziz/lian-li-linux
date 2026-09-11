use anyhow::{bail, Context, Result};
use image::RgbImage;
use lianli_media::pixel_cleaner::{render_frame, FRAME_COUNT};
use std::io::{self, BufWriter, Write};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (width, height) = match args.as_slice() {
        [] => (480_u32, 480_u32),
        [width, height] => (width.parse()?, height.parse()?),
        _ => bail!("usage: pixel_cleaner_preview [width height] (writes RGB24 to stdout)"),
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4096 * 4096 {
        bail!("dimensions must be nonzero and at most 16 megapixels");
    }
    let mut frame = RgbImage::new(width, height);
    let mut output = BufWriter::new(io::stdout().lock());
    for index in 0..FRAME_COUNT {
        render_frame(&mut frame, index);
        output.write_all(frame.as_raw()).context("writing frame")?;
    }
    output.flush().context("flushing frames")
}

use super::*;
use anyhow::{ensure, Result};

pub(super) const MAX_EFFECT_VALUE: u32 = 4;
pub(super) const DEFAULT_SPEED: u32 = 2;
pub(super) const DEFAULT_BRIGHTNESS: u32 = MAX_EFFECT_VALUE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ModeData {
    pub name: String,
    pub value: u32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<[u8; 3]>,
}

impl ModeData {
    pub fn write(&self, buf: &mut Vec<u8>, version: u32) {
        write_string(buf, &self.name);
        if version < 6 {
            buf.extend_from_slice(&self.value.to_le_bytes());
        }
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&self.speed_min.to_le_bytes());
        buf.extend_from_slice(&self.speed_max.to_le_bytes());
        if version >= 3 {
            buf.extend_from_slice(&self.brightness_min.to_le_bytes());
            buf.extend_from_slice(&self.brightness_max.to_le_bytes());
        }
        buf.extend_from_slice(&self.colors_min.to_le_bytes());
        buf.extend_from_slice(&self.colors_max.to_le_bytes());
        buf.extend_from_slice(&self.speed.to_le_bytes());
        if version >= 3 {
            buf.extend_from_slice(&self.brightness.to_le_bytes());
        }
        buf.extend_from_slice(&self.direction.to_le_bytes());
        buf.extend_from_slice(&self.color_mode.to_le_bytes());
        write_colors(buf, &self.colors);
    }

    pub(super) fn read(reader: &mut Reader<'_>, version: u32) -> Result<Self> {
        Ok(Self {
            name: reader.string()?.to_owned(),
            value: if version < 6 { reader.number()? } else { 0 },
            flags: reader.number()?,
            speed_min: reader.number()?,
            speed_max: reader.number()?,
            brightness_min: if version >= 3 { reader.number()? } else { 0 },
            brightness_max: if version >= 3 {
                reader.number()?
            } else {
                MAX_EFFECT_VALUE
            },
            colors_min: reader.number()?,
            colors_max: reader.number()?,
            speed: reader.number()?,
            brightness: if version >= 3 {
                reader.number()?
            } else {
                DEFAULT_BRIGHTNESS
            },
            direction: reader.number()?,
            color_mode: reader.number()?,
            colors: reader.colors()?,
        })
    }
}

#[derive(Debug)]
pub(super) enum Command {
    Custom,
    Mode {
        index: usize,
        mode: ModeData,
    },
    ZoneMode {
        zone: usize,
        selection: Option<(usize, ModeData)>,
    },
    Colors(Vec<[u8; 3]>),
    ZoneColors {
        zone: usize,
        colors: Vec<[u8; 3]>,
    },
    SingleColor {
        led: usize,
        color: [u8; 3],
    },
    Resize {
        zone: usize,
        size: u32,
    },
}

impl Command {
    pub fn decode(kind: u32, payload: &[u8], version: u32) -> Result<Option<Self>> {
        let mut reader = Reader::new(payload);
        let command = match kind {
            PKT_SET_CUSTOM_MODE => Self::Custom,
            PKT_UPDATE_MODE | PKT_SAVE_MODE => {
                reader.size()?;
                Self::Mode {
                    index: reader.number()? as usize,
                    mode: ModeData::read(&mut reader, version)?,
                }
            }
            PKT_UPDATE_ZONE_MODE if version >= 6 => {
                reader.size()?;
                let zone = reader.number()? as usize;
                let index = reader.number()? as i32;
                ensure!(index >= -1, "Invalid zone mode");
                let selection = if index == -1 {
                    None
                } else {
                    Some((index as usize, ModeData::read(&mut reader, version)?))
                };
                Self::ZoneMode { zone, selection }
            }
            PKT_UPDATE_LEDS => {
                reader.size()?;
                Self::Colors(reader.colors()?)
            }
            PKT_UPDATE_ZONE_LEDS => {
                reader.size()?;
                Self::ZoneColors {
                    zone: reader.number()? as usize,
                    colors: reader.colors()?,
                }
            }
            PKT_UPDATE_SINGLE_LED => Self::SingleColor {
                led: reader.number()? as usize,
                color: reader.color()?,
            },
            PKT_RESIZE_ZONE => Self::Resize {
                zone: reader.number()? as usize,
                size: reader.number()?,
            },
            _ => return Ok(None),
        };
        ensure!(reader.finished(), "Unexpected SDK payload suffix");
        Ok(Some(command))
    }
}

pub(super) fn write_colors(buf: &mut Vec<u8>, colors: &[[u8; 3]]) {
    buf.extend_from_slice(&(colors.len() as u16).to_le_bytes());
    for color in colors {
        buf.extend_from_slice(color);
        buf.push(0);
    }
}

pub(super) fn sized_packet(data: &[u8]) -> Vec<u8> {
    let mut result = ((data.len() + 4) as u32).to_le_bytes().to_vec();
    result.extend_from_slice(data);
    result
}

pub(super) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn finished(&self) -> bool {
        self.pos == self.data.len()
    }
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("Invalid SDK length"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| anyhow::anyhow!("Truncated SDK payload"))?;
        self.pos = end;
        Ok(bytes)
    }
    pub fn number(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn short(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub fn size(&mut self) -> Result<()> {
        ensure!(
            self.number()? as usize == self.data.len(),
            "Invalid SDK payload size"
        );
        Ok(())
    }
    pub fn string(&mut self) -> Result<&'a str> {
        let length = self.short()? as usize;
        let bytes = self.take(length)?;
        ensure!(bytes.last() == Some(&0), "Invalid SDK string");
        Ok(std::str::from_utf8(&bytes[..length - 1])?)
    }
    pub fn color(&mut self) -> Result<[u8; 3]> {
        let color = self.take(4)?;
        Ok([color[0], color[1], color[2]])
    }
    pub fn colors(&mut self) -> Result<Vec<[u8; 3]>> {
        let count = self.short()? as usize;
        Ok(self
            .take(count * 4)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_fixture_decodes_versioned_fields_and_rejects_truncation() {
        for version in 0..=6 {
            // Independent wire fixture: a single Static mode, no palette.
            let mut data = vec![7, 0, b'S', b't', b'a', b't', b'i', b'c', 0];
            if version < 6 {
                data.extend_from_slice(&9u32.to_le_bytes());
            }
            for value in [16u32, 0, 4] {
                data.extend_from_slice(&value.to_le_bytes());
            }
            if version >= 3 {
                for value in [0u32, 4] {
                    data.extend_from_slice(&value.to_le_bytes());
                }
            }
            for value in [0u32, 0, 2] {
                data.extend_from_slice(&value.to_le_bytes());
            }
            if version >= 3 {
                data.extend_from_slice(&3u32.to_le_bytes());
            }
            for value in [1u32, 2] {
                data.extend_from_slice(&value.to_le_bytes());
            }
            data.extend_from_slice(&0u16.to_le_bytes());
            let mode = ModeData::read(&mut Reader::new(&data), version).unwrap();
            assert_eq!(mode.name, "Static");
            assert_eq!(mode.value, if version < 6 { 9 } else { 0 });
            assert_eq!(mode.speed, 2);
            assert_eq!(mode.brightness, if version >= 3 { 3 } else { 4 });
            let mut encoded = Vec::new();
            mode.write(&mut encoded, version);
            assert_eq!(encoded, data);
            for end in 0..data.len() {
                assert!(ModeData::read(&mut Reader::new(&data[..end]), version).is_err());
            }
        }
    }
}

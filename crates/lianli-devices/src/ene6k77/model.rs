/// ENE 6K77 model variant, determined by PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ene6k77Model {
    /// 0xA100 — SL Fan (4 groups, 4 fans max each)
    SlFan,
    /// 0xA101 — AL Fan (4 groups, dual-ring LEDs)
    AlFan,
    /// 0xA102 — SL Infinity (4 groups)
    SlInfinity,
    /// 0xA103 — SL V2 Fan (4 groups, 6 fans max each)
    SlV2Fan,
    /// 0xA104 — AL V2 Fan (4 groups, 6 fans max each)
    AlV2Fan,
    /// 0xA105 — SL V2A Fan
    SlV2aFan,
    /// 0xA106 — SL Redragon
    SlRedragon,
}

impl Ene6k77Model {
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0xA100 => Some(Self::SlFan),
            0xA101 => Some(Self::AlFan),
            0xA102 => Some(Self::SlInfinity),
            0xA103 => Some(Self::SlV2Fan),
            0xA104 => Some(Self::AlV2Fan),
            0xA105 => Some(Self::SlV2aFan),
            0xA106 => Some(Self::SlRedragon),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SlFan => "SL Fan",
            Self::AlFan => "AL Fan",
            Self::SlInfinity => "SL Infinity",
            Self::SlV2Fan => "SL V2 Fan",
            Self::AlV2Fan => "AL V2 Fan",
            Self::SlV2aFan => "SL V2A Fan",
            Self::SlRedragon => "SL Redragon",
        }
    }

    /// Whether this is a V2 model (supports 6 fans/group, 9-byte RPM response).
    pub fn is_v2(&self) -> bool {
        matches!(self, Self::SlV2Fan | Self::AlV2Fan | Self::SlV2aFan)
    }

    /// Whether this model uses doubled port encoding (0x10|(group*2) for effects).
    pub fn uses_double_port(&self) -> bool {
        matches!(self, Self::AlFan | Self::AlV2Fan | Self::SlInfinity)
    }

    /// Max fans per group.
    pub fn max_fans_per_group(&self) -> u8 {
        if self.is_v2() {
            6
        } else {
            4
        }
    }

    pub fn palette_size(&self) -> usize {
        match self {
            Self::AlV2Fan => 6,
            _ => 4,
        }
    }

    pub fn single_ring_leds_per_fan(&self) -> usize {
        match self {
            Self::SlFan | Self::SlV2Fan | Self::SlV2aFan | Self::SlRedragon => 16,
            _ => 0,
        }
    }

    pub fn inner_leds_per_fan(&self) -> usize {
        if self.uses_double_port() {
            8
        } else {
            0
        }
    }

    pub fn outer_leds_per_fan(&self) -> usize {
        if self.uses_double_port() {
            12
        } else {
            0
        }
    }

    /// Frame commit value for `[REPORT_ID, 0x60, hi, lo]`. SLV2/SLV2A use 4;
    /// every other variant uses 1.
    pub fn frame_commit_value(&self) -> u16 {
        match self {
            Self::SlV2Fan | Self::SlV2aFan => 4,
            _ => 1,
        }
    }

    /// Expected `(MajorID, MinorID)` pairs from the firmware-version response.
    /// Some variants accept multiple MinorIDs (e.g. SLV2Fan accepts 0xC5 and 0xC7).
    pub fn expected_firmware_ids(&self) -> &'static [(u8, u8)] {
        match self {
            Self::SlFan => &[(0x64, 0xC2)],
            Self::SlRedragon => &[(0x64, 0xC8)],
            Self::AlFan => &[(0x80, 0xC3)],
            Self::SlInfinity => &[(0x80, 0xC4)],
            Self::SlV2Fan | Self::SlV2aFan => &[(0x64, 0xC5), (0x64, 0xC7)],
            Self::AlV2Fan => &[(0x80, 0xC6)],
        }
    }
}

/// Firmware version info read from the device.
#[derive(Debug, Clone)]
pub struct Ene6k77Firmware {
    pub model: Ene6k77Model,
    pub customer_id: u8,
    pub project_id: u8,
    pub major_id: u8,
    pub minor_id: u8,
    pub fine_tune: u8,
}

impl Ene6k77Firmware {
    /// Validate the firmware-ID bytes against the expected values for this
    /// variant: `CustomerID==0xE0 && ProjectID==0x50` plus per-variant
    /// `(MajorID, MinorID)`.
    ///
    /// Returns `false` on mismatch; the caller should log a warning but
    /// not fail (the device may be running newer firmware).
    pub fn is_valid(&self) -> bool {
        if self.customer_id != 0xE0 || self.project_id != 0x50 {
            return false;
        }
        self.model
            .expected_firmware_ids()
            .iter()
            .any(|&(maj, min)| maj == self.major_id && min == self.minor_id)
    }

    /// Per-variant version number as `(major, minor)`.
    pub fn version(&self) -> (u32, u32) {
        let hi = (self.fine_tune >> 4) as u32;
        let lo = (self.fine_tune & 0x0F) as u32;

        match self.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => {
                if lo > 9 {
                    return (0, 0);
                }
                let n = hi * 10 + lo;
                (n / 10, n % 10)
            }
            Ene6k77Model::AlFan => {
                if lo > 9 {
                    return (0, 0);
                }
                if self.fine_tune < 8 {
                    (1, 0)
                } else {
                    let n = hi * 10 + lo + 2;
                    (n / 10, n % 10)
                }
            }
            Ene6k77Model::SlInfinity => {
                if self.fine_tune <= 3 || (15..=31).contains(&self.fine_tune) {
                    return (0, 0);
                }
                if self.fine_tune >= 32 && lo > 9 {
                    return (0, 0);
                }
                let lo_adj = if self.fine_tune == 13 || self.fine_tune == 14 {
                    lo + 1
                } else {
                    lo
                };
                let n = hi * 10 + lo_adj;
                (n / 10, n % 10)
            }
            Ene6k77Model::AlV2Fan => {
                if lo > 9 {
                    return (0, 0);
                }
                let n = hi * 10 + lo;
                (n / 10, n % 10)
            }
            Ene6k77Model::SlV2Fan | Ene6k77Model::SlV2aFan => {
                if lo > 9 {
                    return (0, 0);
                }
                if self.minor_id == 199 {
                    if self.fine_tune == 0 {
                        return (0, 5);
                    }
                    if self.fine_tune <= 5 {
                        return (0, 0);
                    }
                }
                let n = hi * 10 + lo;
                (n / 10, n % 10)
            }
        }
    }
}

impl std::fmt::Display for Ene6k77Firmware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (major, minor) = self.version();
        write!(f, "v{major}.{minor}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fw(model: Ene6k77Model, minor_id: u8, fine_tune: u8) -> Ene6k77Firmware {
        Ene6k77Firmware {
            model,
            customer_id: 0xE0,
            project_id: 0x50,
            major_id: if model.uses_double_port() { 0x80 } else { 0x64 },
            minor_id,
            fine_tune,
        }
    }

    #[test]
    fn slfan_plain_no_plus2() {
        // FineTune 0x42 → hi=4, lo=2 → 42 → v4.2 (NOT v4.4 with +2)
        assert_eq!(fw(Ene6k77Model::SlFan, 0xC2, 0x42).version(), (4, 2));
    }

    #[test]
    fn alfan_plus2_only_above_8() {
        // FineTune < 8 → always v1.0
        assert_eq!(fw(Ene6k77Model::AlFan, 0xC3, 0x05).version(), (1, 0));
        assert_eq!(fw(Ene6k77Model::AlFan, 0xC3, 0x07).version(), (1, 0));
        // FineTune ≥ 8 → hi*10+lo+2
        // 0x08 → hi=0,lo=8 → 0+8+2=10 → v1.0
        assert_eq!(fw(Ene6k77Model::AlFan, 0xC3, 0x08).version(), (1, 0));
        // 0x42 → hi=4,lo=2 → 42+2=44 → v4.4
        assert_eq!(fw(Ene6k77Model::AlFan, 0xC3, 0x42).version(), (4, 4));
    }

    #[test]
    fn slinfinity_conditional_plus1() {
        // FineTune ≤3 → v0.0
        assert_eq!(fw(Ene6k77Model::SlInfinity, 0xC4, 0x03).version(), (0, 0));
        // FineTune 13 (0x0D) → lo=13, lo+1=14 → 14 → v1.4
        assert_eq!(fw(Ene6k77Model::SlInfinity, 0xC4, 13).version(), (1, 4));
        // FineTune 14 (0x0E) → lo=14, lo+1=15 → 15 → v1.5
        assert_eq!(fw(Ene6k77Model::SlInfinity, 0xC4, 14).version(), (1, 5));
        // FineTune 12 (0x0C) → hi=0,lo=12, no bump → 12 → v1.2
        assert_eq!(fw(Ene6k77Model::SlInfinity, 0xC4, 12).version(), (1, 2));
        // FineTune 15 → v0.0
        assert_eq!(fw(Ene6k77Model::SlInfinity, 0xC4, 15).version(), (0, 0));
    }

    #[test]
    fn alv2fan_plain_no_plus2() {
        // 0x42 → hi=4, lo=2 → 42 → v4.2 (NOT v4.4)
        assert_eq!(fw(Ene6k77Model::AlV2Fan, 0xC6, 0x42).version(), (4, 2));
    }

    #[test]
    fn slv2fan_minor199_special() {
        // MinorID=199, FineTune=0 → v0.5
        assert_eq!(fw(Ene6k77Model::SlV2Fan, 199, 0).version(), (0, 5));
        // MinorID=199, FineTune=3 → v0.0
        assert_eq!(fw(Ene6k77Model::SlV2Fan, 199, 3).version(), (0, 0));
        // MinorID=199, FineTune=6 → normal: hi=0,lo=6 → v0.6
        assert_eq!(fw(Ene6k77Model::SlV2Fan, 199, 6).version(), (0, 6));
        // MinorID=197 (normal), FineTune=0x42 → v4.2
        assert_eq!(fw(Ene6k77Model::SlV2Fan, 197, 0x42).version(), (4, 2));
    }

    #[test]
    fn invalid_bcd_returns_zero() {
        // lo nibble > 9 → (0, 0) for all non-SLInfinity variants
        assert_eq!(fw(Ene6k77Model::SlFan, 0xC2, 0x0A).version(), (0, 0));
        assert_eq!(fw(Ene6k77Model::AlFan, 0xC3, 0x0F).version(), (0, 0));
    }

    #[test]
    fn firmware_id_validation() {
        // Valid IDs
        assert!(fw(Ene6k77Model::SlFan, 0xC2, 0x42).is_valid());
        assert!(fw(Ene6k77Model::SlRedragon, 0xC8, 0x42).is_valid());
        assert!(fw(Ene6k77Model::AlFan, 0xC3, 0x42).is_valid());
        assert!(fw(Ene6k77Model::SlInfinity, 0xC4, 0x42).is_valid());
        assert!(fw(Ene6k77Model::AlV2Fan, 0xC6, 0x42).is_valid());
        // SLV2Fan accepts both 0xC5 and 0xC7
        assert!(fw(Ene6k77Model::SlV2Fan, 0xC5, 0x42).is_valid());
        assert!(fw(Ene6k77Model::SlV2Fan, 0xC7, 0x42).is_valid());
        assert!(fw(Ene6k77Model::SlV2aFan, 0xC7, 0x42).is_valid());

        // Invalid customer ID
        let mut bad_customer = fw(Ene6k77Model::SlFan, 0xC2, 0x42);
        bad_customer.customer_id = 0xFF;
        assert!(!bad_customer.is_valid());

        // Wrong MinorID for variant
        let mut mismatched = fw(Ene6k77Model::SlFan, 0xC2, 0x42);
        mismatched.minor_id = 0xC8; // SLRedragon's MinorID
        assert!(!mismatched.is_valid());
    }
}

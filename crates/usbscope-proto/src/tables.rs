//! Lookup tables: time/div IDs, roll-mode rate IDs, vertical scale IDs and the
//! per-channel-count sample-rate factors (all as used by hantek1008py).

use crate::error::ProtoError;

/// Nominal number of samples per horizontal division in burst mode.
///
/// hantek1008py notes "a div contains around 25 samples"; this is an
/// approximation, not a measured constant.
pub const SAMPLES_PER_DIV: f64 = 25.0;

/// Number of analog input channels.
pub const CHANNELS: usize = 8;

/// Nominal ADC reading for 0 V (12-bit mid-scale).
pub const NOMINAL_ZERO_OFFSET: f64 = 2048.0;

/// Volts per ADC count at scale factor 1.0; multiply by
/// [`ScaleId::volts_per_unit`] for the other scales.
pub const VOLTS_PER_COUNT_AT_UNIT_SCALE: f64 = 0.01;

/// Pick the entry of `candidates` closest to `target` by ratio (log distance).
///
/// Avoids `f64::ln`, which is unavailable in `no_std`.
fn nearest_by_ratio(target: f64, candidates: impl Iterator<Item = (u8, f64)>) -> Option<u8> {
    if !target.is_finite() || target <= 0.0 {
        return None;
    }
    let mut best: Option<(u8, f64)> = None;
    for (id, value) in candidates {
        let ratio = if value > target {
            value / target
        } else {
            target / value
        };
        if best.is_none_or(|(_, r)| ratio < r) {
            best = Some((id, ratio));
        }
    }
    best.map(|(id, _)| id)
}

/// Burst-mode time/div ID (`0xA3` parameter), 0..=25.
///
/// ID `n` selects `[1, 2, 5][n % 3] * 10^(n / 3)` ns per division,
/// i.e. 1 ns (ID 0) up to 200 ms (ID 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeDivId(u8);

impl TimeDivId {
    /// Largest valid ID (200 ms/div).
    pub const MAX: u8 = 25;

    /// The vendor software's default, 500 us/div (ID 17).
    pub const DEFAULT: TimeDivId = TimeDivId(17);

    /// Validate a raw ID.
    pub fn new(id: u8) -> Result<Self, ProtoError> {
        if id <= Self::MAX {
            Ok(Self(id))
        } else {
            Err(ProtoError::InvalidTimeDivId(id))
        }
    }

    /// Raw ID byte.
    pub fn id(self) -> u8 {
        self.0
    }

    /// Iterate all valid IDs in ascending order.
    pub fn all() -> impl Iterator<Item = TimeDivId> {
        (0..=Self::MAX).map(TimeDivId)
    }

    /// Nanoseconds per division.
    pub fn nanos_per_div(self) -> u64 {
        const MANTISSA: [u64; 3] = [1, 2, 5];
        MANTISSA[usize::from(self.0 % 3)] * 10u64.pow(u32::from(self.0 / 3))
    }

    /// Seconds per division.
    pub fn seconds_per_div(self) -> f64 {
        self.nanos_per_div() as f64 * 1e-9
    }

    /// Exact lookup by nanoseconds per division.
    pub fn from_nanos(ns: u64) -> Option<Self> {
        Self::all().find(|t| t.nanos_per_div() == ns)
    }

    /// Nearest valid ID for an arbitrary seconds-per-division value
    /// (`None` for non-positive or non-finite input).
    pub fn from_seconds(seconds: f64) -> Option<Self> {
        nearest_by_ratio(seconds, Self::all().map(|t| (t.0, t.seconds_per_div()))).map(TimeDivId)
    }
}

impl Default for TimeDivId {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Roll-mode sample-rate ID (`0xA3` parameter in roll mode), 0x18..=0x24.
///
/// hantek1008py observed a pipe error on `0xC0` in roll mode when `0xA3` was
/// not given one of these IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RollRateId(u8);

impl RollRateId {
    /// First valid ID (440 Hz).
    pub const MIN: u8 = 0x18;
    /// Last valid ID (1/16 Hz).
    pub const MAX: u8 = 0x24;

    const HZ: [f64; 13] = [
        440.0,
        220.0,
        88.0,
        44.0,
        22.0,
        11.0,
        5.0,
        2.0,
        1.0,
        0.5,
        0.25,
        0.125,
        1.0 / 16.0,
    ];

    /// Validate a raw ID.
    pub fn new(id: u8) -> Result<Self, ProtoError> {
        if (Self::MIN..=Self::MAX).contains(&id) {
            Ok(Self(id))
        } else {
            Err(ProtoError::InvalidRollRateId(id))
        }
    }

    /// Raw ID byte.
    pub fn id(self) -> u8 {
        self.0
    }

    /// Iterate all valid IDs, fastest first.
    pub fn all() -> impl Iterator<Item = RollRateId> {
        (Self::MIN..=Self::MAX).map(RollRateId)
    }

    /// Nominal samples per second per channel with all 8 channels active.
    ///
    /// With fewer channels the device samples faster; multiply by
    /// [`sample_rate_factor`].
    pub fn hz(self) -> f64 {
        Self::HZ[usize::from(self.0 - Self::MIN)]
    }

    /// Nearest valid ID for a rate in Hz (`None` for non-positive or
    /// non-finite input).
    pub fn from_hz(hz: f64) -> Option<Self> {
        nearest_by_ratio(hz, Self::all().map(|r| (r.0, r.hz()))).map(RollRateId)
    }
}

impl Default for RollRateId {
    fn default() -> Self {
        Self(Self::MIN)
    }
}

/// Vertical scale ID (`0xA2` parameter, one per channel).
///
/// The ID-to-factor mapping follows hantek1008py, which is the only
/// implementation verified on hardware: ID 1 = 0.02, ID 2 = 0.125, ID 3 = 1.0.
/// (PLAN.md's table lists the reverse order; hantek1008py wins until a USB
/// capture says otherwise.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum ScaleId {
    /// Factor 0.02 (most sensitive range).
    V0_02 = 1,
    /// Factor 0.125.
    V0_125 = 2,
    /// Factor 1.0 (least sensitive range, vendor default).
    #[default]
    V1 = 3,
}

impl ScaleId {
    /// All scales in ID order.
    pub const ALL: [ScaleId; 3] = [ScaleId::V0_02, ScaleId::V0_125, ScaleId::V1];

    /// Validate a raw ID.
    pub fn new(id: u8) -> Result<Self, ProtoError> {
        match id {
            1 => Ok(Self::V0_02),
            2 => Ok(Self::V0_125),
            3 => Ok(Self::V1),
            other => Err(ProtoError::InvalidScaleId(other)),
        }
    }

    /// Raw ID byte.
    pub fn id(self) -> u8 {
        self as u8
    }

    /// Zero-based index (ID - 1), handy for per-scale arrays.
    pub fn index(self) -> usize {
        usize::from(self.id() - 1)
    }

    /// The scale factor hantek1008py calls "vscale" (1.0, 0.125 or 0.02).
    ///
    /// Volts per ADC count is `0.01 * volts_per_unit()`; see
    /// [`ScaleId::volts_per_count`].
    pub fn volts_per_unit(self) -> f64 {
        match self {
            Self::V0_02 => 0.02,
            Self::V0_125 => 0.125,
            Self::V1 => 1.0,
        }
    }

    /// Volts represented by one ADC count at this scale.
    pub fn volts_per_count(self) -> f64 {
        VOLTS_PER_COUNT_AT_UNIT_SCALE * self.volts_per_unit()
    }

    /// Exact lookup by scale factor (tolerates float noise).
    pub fn from_volts_per_unit(factor: f64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| (s.volts_per_unit() - factor).abs() < 1e-9)
    }
}

/// How much faster than nominal the device samples with `channels` active
/// (1..=8); `None` outside that range.
///
/// Empirical values from hantek1008py; 8 channels = 1.0.
pub fn sample_rate_factor(channels: usize) -> Option<f64> {
    const FACTORS: [f64; CHANNELS] = [4.56, 3.03, 2.27, 1.82, 1.51, 1.30, 1.14, 1.00];
    channels
        .checked_sub(1)
        .and_then(|i| FACTORS.get(i))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_div_table() {
        assert_eq!(TimeDivId::new(0).unwrap().nanos_per_div(), 1);
        assert_eq!(TimeDivId::new(1).unwrap().nanos_per_div(), 2);
        assert_eq!(TimeDivId::new(2).unwrap().nanos_per_div(), 5);
        assert_eq!(TimeDivId::new(3).unwrap().nanos_per_div(), 10);
        assert_eq!(TimeDivId::new(17).unwrap().nanos_per_div(), 500_000);
        assert_eq!(TimeDivId::new(25).unwrap().nanos_per_div(), 200_000_000);
        assert!(TimeDivId::new(26).is_err());
        assert_eq!(TimeDivId::default().nanos_per_div(), 500_000);
    }

    #[test]
    fn time_div_round_trip() {
        for t in TimeDivId::all() {
            assert_eq!(TimeDivId::from_seconds(t.seconds_per_div()), Some(t));
            assert_eq!(TimeDivId::from_nanos(t.nanos_per_div()), Some(t));
        }
    }

    #[test]
    fn time_div_nearest() {
        // 1 ms is exact (ID 18); 1.4 ms rounds to 1 ms, 1.5 ms to 2 ms.
        assert_eq!(TimeDivId::from_seconds(1e-3).unwrap().id(), 18);
        assert_eq!(TimeDivId::from_seconds(1.4e-3).unwrap().id(), 18);
        assert_eq!(TimeDivId::from_seconds(1.5e-3).unwrap().id(), 19);
        assert_eq!(TimeDivId::from_seconds(10.0).unwrap().id(), 25);
        assert_eq!(TimeDivId::from_seconds(1e-12).unwrap().id(), 0);
        assert_eq!(TimeDivId::from_seconds(0.0), None);
        assert_eq!(TimeDivId::from_seconds(f64::NAN), None);
        assert_eq!(TimeDivId::from_nanos(3), None);
    }

    #[test]
    fn roll_rate_table() {
        assert_eq!(RollRateId::new(0x18).unwrap().hz(), 440.0);
        assert_eq!(RollRateId::new(0x1e).unwrap().hz(), 5.0);
        assert_eq!(RollRateId::new(0x24).unwrap().hz(), 0.0625);
        assert!(RollRateId::new(0x17).is_err());
        assert!(RollRateId::new(0x25).is_err());
        assert_eq!(RollRateId::all().count(), 13);
        for r in RollRateId::all() {
            assert_eq!(RollRateId::from_hz(r.hz()), Some(r));
        }
        assert_eq!(RollRateId::from_hz(400.0).unwrap().id(), 0x18);
    }

    #[test]
    fn scale_ids() {
        for s in ScaleId::ALL {
            assert_eq!(ScaleId::new(s.id()), Ok(s));
            assert_eq!(ScaleId::from_volts_per_unit(s.volts_per_unit()), Some(s));
        }
        assert_eq!(ScaleId::V0_02.id(), 1);
        assert_eq!(ScaleId::V1.id(), 3);
        assert_eq!(ScaleId::V1.index(), 2);
        assert!((ScaleId::V1.volts_per_count() - 0.01).abs() < 1e-12);
        assert!(ScaleId::new(0).is_err());
        assert!(ScaleId::new(4).is_err());
        assert_eq!(ScaleId::from_volts_per_unit(0.5), None);
    }

    #[test]
    fn sample_rate_factors() {
        assert_eq!(sample_rate_factor(1), Some(4.56));
        assert_eq!(sample_rate_factor(8), Some(1.0));
        assert_eq!(sample_rate_factor(0), None);
        assert_eq!(sample_rate_factor(9), None);
    }
}

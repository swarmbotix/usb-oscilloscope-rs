//! Sample decoding: bytes to 16-bit samples, de-interleaving, and raw to volts.

use alloc::vec::Vec;

use crate::tables::{CHANNELS, NOMINAL_ZERO_OFFSET, ScaleId};

/// Decode little-endian 16-bit samples. A trailing odd byte is ignored
/// (the device always reports even byte counts).
pub fn decode_u16_le(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// Split burst-mode samples interleaved across `channels` active channels
/// (ascending channel order) into one vector per channel.
///
/// A trailing partial frame is dropped so all vectors have equal length.
/// Returns an empty vector when `channels` is 0.
pub fn deinterleave(samples: &[u16], channels: usize) -> Vec<Vec<u16>> {
    deinterleave_stride(samples, channels, channels)
}

/// Like [`deinterleave`], for roll mode: each frame carries one extra
/// trailing value after the active channels (a "9th channel" reading around
/// 1742 whose meaning is unknown), which is dropped.
pub fn deinterleave_roll(samples: &[u16], channels: usize) -> Vec<Vec<u16>> {
    deinterleave_stride(samples, channels, channels + 1)
}

fn deinterleave_stride(samples: &[u16], keep: usize, stride: usize) -> Vec<Vec<u16>> {
    if keep == 0 {
        return Vec::new();
    }
    let frames = samples.len() / stride;
    let mut out: Vec<Vec<u16>> = (0..keep).map(|_| Vec::with_capacity(frames)).collect();
    for frame in samples.chunks_exact(stride) {
        for (ch, &value) in out.iter_mut().zip(frame) {
            ch.push(value);
        }
    }
    out
}

/// Convert a raw ADC reading to volts: `(raw - zero_offset) * 0.01 * factor`.
pub fn raw_to_volts(raw: u16, zero_offset: f64, scale: ScaleId) -> f64 {
    (f64::from(raw) - zero_offset) * scale.volts_per_count()
}

/// Arithmetic mean of a sample run, `None` if empty.
pub fn mean(samples: &[u16]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let sum: u64 = samples.iter().map(|&s| u64::from(s)).sum();
    Some(sum as f64 / samples.len() as f64)
}

/// Per-scale, per-channel zero offsets in raw ADC counts.
///
/// The vendor init measures these by capturing with every scale and averaging
/// each channel (inputs are assumed idle at that moment).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroOffsets {
    values: [[f64; CHANNELS]; 3],
}

impl Default for ZeroOffsets {
    fn default() -> Self {
        Self {
            values: [[NOMINAL_ZERO_OFFSET; CHANNELS]; 3],
        }
    }
}

impl ZeroOffsets {
    /// Offset for `channel` (0..=7) at `scale`; nominal 2048 for an
    /// out-of-range channel.
    pub fn get(&self, scale: ScaleId, channel: usize) -> f64 {
        self.values[scale.index()]
            .get(channel)
            .copied()
            .unwrap_or(NOMINAL_ZERO_OFFSET)
    }

    /// Set the offset for `channel` (0..=7) at `scale`; out-of-range channels
    /// are ignored.
    pub fn set(&mut self, scale: ScaleId, channel: usize, value: f64) {
        if let Some(slot) = self.values[scale.index()].get_mut(channel) {
            *slot = value;
        }
    }

    /// All eight offsets for one scale.
    pub fn for_scale(&self, scale: ScaleId) -> &[f64; CHANNELS] {
        &self.values[scale.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn decode_le() {
        assert_eq!(
            decode_u16_le(&[0x00, 0x08, 0xd4, 0x07, 0xff]),
            [0x0800, 0x07d4]
        );
        assert!(decode_u16_le(&[]).is_empty());
    }

    #[test]
    fn deinterleave_burst() {
        let s = [10, 20, 11, 21, 12, 22, 13];
        assert_eq!(
            deinterleave(&s, 2),
            vec![vec![10, 11, 12], vec![20, 21, 22]]
        );
        assert_eq!(deinterleave(&s, 1), vec![s.to_vec()]);
        assert!(deinterleave(&s, 0).is_empty());
        let eight: Vec<u16> = (0..16).collect();
        let per = deinterleave(&eight, 8);
        assert_eq!(per[0], [0, 8]);
        assert_eq!(per[7], [7, 15]);
    }

    #[test]
    fn deinterleave_roll_drops_ninth() {
        // Two active channels + one extra value per frame.
        let s = [10, 20, 1742, 11, 21, 1741];
        assert_eq!(deinterleave_roll(&s, 2), vec![vec![10, 11], vec![20, 21]]);
    }

    #[test]
    fn volts() {
        assert_eq!(raw_to_volts(2048, 2048.0, ScaleId::V1), 0.0);
        assert!((raw_to_volts(2148, 2048.0, ScaleId::V1) - 1.0).abs() < 1e-12);
        assert!((raw_to_volts(1948, 2048.0, ScaleId::V0_02) + 0.02).abs() < 1e-12);
        assert!((raw_to_volts(2148, 2048.0, ScaleId::V0_125) - 0.125).abs() < 1e-12);
    }

    #[test]
    fn zero_offsets() {
        assert_eq!(mean(&[1, 2, 3, 4]), Some(2.5));
        assert_eq!(mean(&[]), None);
        let mut z = ZeroOffsets::default();
        assert_eq!(z.get(ScaleId::V1, 3), 2048.0);
        z.set(ScaleId::V0_02, 3, 2050.5);
        assert_eq!(z.get(ScaleId::V0_02, 3), 2050.5);
        assert_eq!(z.get(ScaleId::V1, 3), 2048.0);
        assert_eq!(z.get(ScaleId::V1, 99), 2048.0);
        assert_eq!(z.for_scale(ScaleId::V0_02)[3], 2050.5);
    }
}

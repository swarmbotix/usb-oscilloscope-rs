//! Acquisition settings and the command sequence that applies them.

use alloc::vec::Vec;

use crate::cmd::{AC_CONFIGURE, Command, Slope};
use crate::error::ProtoError;
use crate::tables::{CHANNELS, ScaleId, TimeDivId};

/// Largest 12-bit trigger level.
pub const MAX_TRIGGER_LEVEL: u16 = 0x0fff;

/// Trigger settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Trigger {
    /// Zero-based source channel (0..=7).
    pub channel: u8,
    /// Edge direction.
    pub slope: Slope,
    /// Raw 12-bit ADC level (2048 = mid-scale).
    pub level: u16,
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            channel: 0,
            slope: Slope::Rising,
            level: 2048,
        }
    }
}

/// Channel, timebase and trigger configuration.
///
/// The default matches hantek1008py's defaults: all channels on, scale 1.0,
/// 500 us/div, rising-edge trigger on channel 0 at mid-scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Config {
    /// Enabled channels, index 0 = CH1.
    pub channels: [bool; CHANNELS],
    /// Vertical scale per channel (sent for disabled channels too).
    pub scales: [ScaleId; CHANNELS],
    /// Burst-mode time/div.
    pub time_div: TimeDivId,
    /// Trigger settings.
    pub trigger: Trigger,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channels: [true; CHANNELS],
            scales: [ScaleId::default(); CHANNELS],
            time_div: TimeDivId::default(),
            trigger: Trigger::default(),
        }
    }
}

impl Config {
    /// Zero-based indices of enabled channels, ascending. Samples arrive
    /// interleaved in this order.
    pub fn active_channels(&self) -> Vec<usize> {
        (0..CHANNELS).filter(|&i| self.channels[i]).collect()
    }

    /// Number of enabled channels.
    pub fn active_count(&self) -> usize {
        self.channels.iter().filter(|&&on| on).count()
    }

    /// Reject settings the device cannot represent.
    pub fn validate(&self) -> Result<(), ProtoError> {
        let n = self.active_count();
        if n == 0 {
            return Err(ProtoError::InvalidChannelCount(n));
        }
        if usize::from(self.trigger.channel) >= CHANNELS {
            return Err(ProtoError::InvalidChannel(usize::from(
                self.trigger.channel,
            )));
        }
        if self.trigger.level > MAX_TRIGGER_LEVEL {
            return Err(ProtoError::TriggerLevelOutOfRange(self.trigger.level));
        }
        Ok(())
    }

    /// Requests that apply this configuration, in hantek1008py's order
    /// (`A0`, `AA`, `A2`, `A3`, `C1`, `A7`, `AC`, `AB`). The `A7` reply must be
    /// checked with [`crate::cmd::parse_unknown7`].
    pub fn commands(&self) -> [Command; 8] {
        [
            Command::ChannelCount(self.active_count() as u8),
            Command::ChannelMap(self.channels),
            Command::VerticalScale(self.scales),
            Command::TimeDiv(self.time_div),
            Command::TriggerConfig {
                channel: self.trigger.channel,
                slope: self.trigger.slope,
            },
            Command::Unknown7,
            Command::AcConfig(AC_CONFIGURE),
            Command::TriggerLevel(self.trigger.level),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::PACKET_SIZE;

    #[test]
    fn default_config_bytes() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        let mut out = Vec::new();
        for cmd in cfg.commands() {
            let mut buf = [0u8; PACKET_SIZE];
            let n = cmd.encode(&mut buf);
            out.extend_from_slice(&buf[..n]);
        }
        let expected: &[u8] = &[
            0xa0, 0x08, //
            0xaa, 1, 1, 1, 1, 1, 1, 1, 1, //
            0xa2, 3, 3, 3, 3, 3, 3, 3, 3, //
            0xa3, 0x11, //
            0xc1, 0x00, 0x00, //
            0xa7, 0x00, 0x00, //
            0xac, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x05, 0x79, //
            0xab, 0x08, 0x00,
        ];
        assert_eq!(out, expected);
    }

    #[test]
    fn active_channels() {
        let cfg = Config {
            channels: [false, true, false, false, true, false, false, false],
            ..Config::default()
        };
        assert_eq!(cfg.active_channels(), [1, 4]);
        assert_eq!(cfg.active_count(), 2);
        assert_eq!(cfg.commands()[0], Command::ChannelCount(2));
    }

    #[test]
    fn validation() {
        let cfg = Config {
            channels: [false; CHANNELS],
            ..Config::default()
        };
        assert_eq!(cfg.validate(), Err(ProtoError::InvalidChannelCount(0)));
        let mut cfg = Config::default();
        cfg.trigger.channel = 8;
        assert_eq!(cfg.validate(), Err(ProtoError::InvalidChannel(8)));
        let mut cfg = Config::default();
        cfg.trigger.level = 4096;
        assert_eq!(
            cfg.validate(),
            Err(ProtoError::TriggerLevelOutOfRange(4096))
        );
    }
}

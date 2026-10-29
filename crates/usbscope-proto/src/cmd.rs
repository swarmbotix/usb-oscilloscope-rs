//! Request encoding and reply validation for every known command.
//!
//! Every request is `[cmd, params...]` sent in one bulk OUT packet. Most
//! replies start with an echo of `cmd`; the raw data reads listed in
//! [`Command::has_echo`] do not. Reply lengths and echo flags mirror
//! hantek1008py exactly, since that is the only implementation known to work.

use core::time::Duration;

use crate::error::ProtoError;
use crate::tables::{CHANNELS, RollRateId, ScaleId, TimeDivId};

/// USB vendor ID of the Hantek 1008/1008C.
pub const VENDOR_ID: u16 = 0x0783;
/// USB product ID of the Hantek 1008/1008C.
pub const PRODUCT_ID: u16 = 0x5725;
/// Bulk packet size (full-speed); every request fits in one packet.
pub const PACKET_SIZE: usize = 64;

/// Raw command bytes.
pub mod id {
    #![allow(missing_docs)]
    pub const CHANNEL_COUNT: u8 = 0xA0;
    pub const VERTICAL_SCALE: u8 = 0xA2;
    pub const TIME_DIV: u8 = 0xA3;
    pub const MODE: u8 = 0xA4;
    pub const READY_CHECK: u8 = 0xA5;
    pub const BURST_CHUNK: u8 = 0xA6;
    pub const UNKNOWN_7: u8 = 0xA7;
    pub const CHANNEL_MAP: u8 = 0xAA;
    pub const TRIGGER_LEVEL: u8 = 0xAB;
    pub const AC_CONFIG: u8 = 0xAC;
    pub const INIT: u8 = 0xB0;
    pub const BLOB_B5: u8 = 0xB5;
    pub const BLOB_B6: u8 = 0xB6;
    pub const GENERATOR_CONTROL: u8 = 0xB7;
    pub const GENERATOR_WAVEFORM: u8 = 0xB8;
    pub const GENERATOR_PULSE_LENGTH: u8 = 0xB9;
    pub const GENERATOR_OUTPUT: u8 = 0xBB;
    pub const GENERATOR_WAVEFORM_LENGTH: u8 = 0xBF;
    pub const ARM: u8 = 0xC0;
    pub const TRIGGER_CONFIG: u8 = 0xC1;
    pub const FORCE_TRIGGER: u8 = 0xC2;
    pub const BURST_LENGTH: u8 = 0xC6;
    pub const ROLL_READY: u8 = 0xC7;
    pub const ROLL_CHUNK: u8 = 0xC8;
    pub const UNKNOWN_E4: u8 = 0xE4;
    pub const CAL_TEMPERATURE: u8 = 0xE5;
    pub const UNKNOWN_E6: u8 = 0xE6;
    pub const FIRMWARE_VERSION: u8 = 0xE9;
    pub const PING: u8 = 0xF3;
    pub const CLOSE: u8 = 0xF4;
    pub const INIT_STEP_5: u8 = 0xF5;
    pub const INIT_STEP_6: u8 = 0xF6;
    pub const BLOB_F7: u8 = 0xF7;
    pub const BLOB_F8: u8 = 0xF8;
    pub const BLOB_FA: u8 = 0xFA;
}

/// Calibration/config blobs read during init. Contents are device-specific
/// and not yet decoded; they are reported verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InitBlob {
    /// `0xB5`, 64 bytes.
    B5,
    /// `0xB6`, 64 bytes.
    B6,
    /// `0xF7`, 64 bytes.
    F7,
    /// `0xF8`, 64 bytes.
    F8,
    /// `0xFA`, 56 bytes.
    Fa,
}

impl InitBlob {
    /// All blobs in the order the vendor init reads them.
    pub const ALL: [InitBlob; 5] = [
        InitBlob::B5,
        InitBlob::B6,
        InitBlob::F7,
        InitBlob::F8,
        InitBlob::Fa,
    ];

    /// Command byte.
    pub fn id(self) -> u8 {
        match self {
            Self::B5 => id::BLOB_B5,
            Self::B6 => id::BLOB_B6,
            Self::F7 => id::BLOB_F7,
            Self::F8 => id::BLOB_F8,
            Self::Fa => id::BLOB_FA,
        }
    }

    /// Reply length in bytes (no echo).
    pub fn reply_len(self) -> usize {
        match self {
            Self::Fa => 56,
            _ => 64,
        }
    }
}

/// Acquisition mode (`0xA4` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Mode {
    /// Triggered capture of one buffer.
    Burst = 1,
    /// Continuous slow streaming.
    Roll = 2,
}

/// Burst sample buffer selector (`0xC6`/`0xA6` parameter). A full capture is
/// block 2 followed by block 3, concatenated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Block {
    /// First half, parameter 2.
    Two = 2,
    /// Second half, parameter 3.
    Three = 3,
}

impl Block {
    /// Blocks in read order.
    pub const ALL: [Block; 2] = [Block::Two, Block::Three];
}

/// Trigger slope (`0xC1` second parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Slope {
    /// Trigger on a rising edge.
    #[default]
    Rising = 0,
    /// Trigger on a falling edge.
    Falling = 1,
}

/// An arbitrary request for probing undocumented commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawCommand {
    cmd: u8,
    params: [u8; PACKET_SIZE - 1],
    params_len: u8,
    reply_len: u16,
    echo: bool,
}

impl RawCommand {
    /// Build a raw command. `reply_len` counts the echo byte when `echo` is set.
    pub fn new(cmd: u8, params: &[u8], reply_len: usize, echo: bool) -> Result<Self, ProtoError> {
        if params.len() > PACKET_SIZE - 1 {
            return Err(ProtoError::ParamsTooLong(params.len()));
        }
        if reply_len == 0 {
            return Err(ProtoError::ZeroReplyLength);
        }
        let mut buf = [0u8; PACKET_SIZE - 1];
        buf[..params.len()].copy_from_slice(params);
        Ok(Self {
            cmd,
            params: buf,
            params_len: params.len() as u8,
            reply_len: u16::try_from(reply_len).unwrap_or(u16::MAX),
            echo,
        })
    }

    /// Command byte.
    pub fn cmd(&self) -> u8 {
        self.cmd
    }

    /// Parameter bytes.
    pub fn params(&self) -> &[u8] {
        &self.params[..usize::from(self.params_len)]
    }
}

/// A request to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// `B0`: init/reset. The vendor sends it twice, 0.7 s apart.
    Init,
    /// `E9`: firmware version, 2 raw bytes (hantek1008py sees `01 09`).
    FirmwareVersion,
    /// `E5`: calibration temperature, 2 raw bytes.
    CalTemperature,
    /// `B5`/`B6`/`F7`/`F8`/`FA`: read an init-time blob.
    InitBlob(InitBlob),
    /// `F5`: init step; the device needs ~0.2 s before the reply is read.
    InitStep5,
    /// `F6`: init step; same latency as `F5`.
    InitStep6,
    /// `F3`: keep-alive, sent every ~10 ms while idle by the vendor app.
    Ping,
    /// `F4`: close session.
    Close,
    /// `A0 n`: number of active channels (1..=8).
    ChannelCount(u8),
    /// `AA m1..m8`: channel enable map.
    ChannelMap([bool; CHANNELS]),
    /// `A2 s1..s8`: vertical scale per channel.
    VerticalScale([ScaleId; CHANNELS]),
    /// `A3 id`: burst-mode time/div.
    TimeDiv(TimeDivId),
    /// `A3 id`: roll-mode sample rate.
    RollRate(RollRateId),
    /// `A4 m`: acquisition mode.
    Mode(Mode),
    /// `A5 5A`: ready check; payload 2 or 3 means a capture is ready.
    ReadyCheck,
    /// `A7 00 00`: purpose unknown; payload must be `00`.
    Unknown7,
    /// `AC` + 8 bytes: undecoded config blob; the init uses three fixed values
    /// ([`AC_INIT_1`], [`AC_INIT_2`], [`AC_CONFIGURE`]).
    AcConfig([u8; 8]),
    /// `AB hi lo`: trigger level, 12-bit big-endian (masked to 12 bits).
    TriggerLevel(u16),
    /// `C1 ch slope`: trigger source channel (0..=7) and slope.
    TriggerConfig {
        /// Zero-based source channel.
        channel: u8,
        /// Edge direction.
        slope: Slope,
    },
    /// `C0`: arm.
    Arm,
    /// `C2`: force trigger.
    ForceTrigger,
    /// `C6 b`: byte count of burst block `b`, 2-byte big-endian, no echo.
    BurstLength(Block),
    /// `A6 b`: next 64-byte chunk of burst block `b`, no echo.
    BurstChunk(Block),
    /// `C7`: roll-mode bytes ready, 2-byte big-endian, no echo.
    RollReady,
    /// `C8`: next 64-byte roll-mode chunk, no echo.
    RollChunk,
    /// `E4 x`: purpose unknown; hantek1008py sends `E4 01` around captures.
    UnknownE4(u8),
    /// `E6 x`: 10 raw bytes, no echo. Looks like five little-endian u16
    /// readings near the `E5` value (e.g. `e9 06 e5 06 ...`).
    UnknownE6(u8),
    /// `B7 x`: generator control; hantek1008py sends `B7 00` before
    /// reconfiguring the generator.
    GeneratorControl(u8),
    /// `BB 08 x`: generator output enable.
    GeneratorOutput(bool),
    /// `B9 01 len32le`: generator pulse length (vendor init sends 1200).
    GeneratorPulseLength(u32),
    /// `BF lo hi`: generator waveform length in bytes.
    GeneratorWaveformLength(u16),
    /// `B8 page data[62]`: one generator waveform page. hantek1008py only
    /// supports a single page and sends `page = 1`.
    GeneratorWaveform {
        /// Page byte (hantek1008py: always 1).
        page: u8,
        /// One bit per digital output per step.
        data: [u8; 62],
    },
    /// Arbitrary command for experiments.
    Raw(RawCommand),
}

/// `AC` blob sent in the first init phase.
pub const AC_INIT_1: [u8; 8] = [0x01, 0xf4, 0x00, 0x09, 0xc5, 0x00, 0x09, 0xc5];
/// `AC` blob sent after `F6` in the last init phase.
pub const AC_INIT_2: [u8; 8] = [0x00, 0xc8, 0x00, 0x02, 0xbd, 0x00, 0x02, 0xbd];
/// `AC` blob sent at the end of channel/trigger configuration.
pub const AC_CONFIGURE: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x05, 0x79];

/// Pulse length the vendor init programs into the generator (300 000 rpm in
/// hantek1008py's terms).
pub const DEFAULT_GENERATOR_PULSE_LENGTH: u32 = 1200;

/// Latency the device needs between `A2`/`F5`/`F6` and reading the reply.
const SLOW_REPLY: Duration = Duration::from_micros(213_200);

impl Command {
    /// Command byte.
    pub fn id(&self) -> u8 {
        match self {
            Self::Init => id::INIT,
            Self::FirmwareVersion => id::FIRMWARE_VERSION,
            Self::CalTemperature => id::CAL_TEMPERATURE,
            Self::InitBlob(b) => b.id(),
            Self::InitStep5 => id::INIT_STEP_5,
            Self::InitStep6 => id::INIT_STEP_6,
            Self::Ping => id::PING,
            Self::Close => id::CLOSE,
            Self::ChannelCount(_) => id::CHANNEL_COUNT,
            Self::ChannelMap(_) => id::CHANNEL_MAP,
            Self::VerticalScale(_) => id::VERTICAL_SCALE,
            Self::TimeDiv(_) | Self::RollRate(_) => id::TIME_DIV,
            Self::Mode(_) => id::MODE,
            Self::ReadyCheck => id::READY_CHECK,
            Self::Unknown7 => id::UNKNOWN_7,
            Self::AcConfig(_) => id::AC_CONFIG,
            Self::TriggerLevel(_) => id::TRIGGER_LEVEL,
            Self::TriggerConfig { .. } => id::TRIGGER_CONFIG,
            Self::Arm => id::ARM,
            Self::ForceTrigger => id::FORCE_TRIGGER,
            Self::BurstLength(_) => id::BURST_LENGTH,
            Self::BurstChunk(_) => id::BURST_CHUNK,
            Self::RollReady => id::ROLL_READY,
            Self::RollChunk => id::ROLL_CHUNK,
            Self::UnknownE4(_) => id::UNKNOWN_E4,
            Self::UnknownE6(_) => id::UNKNOWN_E6,
            Self::GeneratorControl(_) => id::GENERATOR_CONTROL,
            Self::GeneratorOutput(_) => id::GENERATOR_OUTPUT,
            Self::GeneratorPulseLength(_) => id::GENERATOR_PULSE_LENGTH,
            Self::GeneratorWaveformLength(_) => id::GENERATOR_WAVEFORM_LENGTH,
            Self::GeneratorWaveform { .. } => id::GENERATOR_WAVEFORM,
            Self::Raw(r) => r.cmd,
        }
    }

    /// Write the request into `out` and return its length.
    pub fn encode(&self, out: &mut [u8; PACKET_SIZE]) -> usize {
        out[0] = self.id();
        let params = &mut out[1..];
        let n = match self {
            Self::Init
            | Self::FirmwareVersion
            | Self::CalTemperature
            | Self::InitBlob(_)
            | Self::InitStep5
            | Self::InitStep6
            | Self::Ping
            | Self::Close
            | Self::Arm
            | Self::ForceTrigger
            | Self::RollReady
            | Self::RollChunk => 0,
            Self::ChannelCount(n) => put(params, &[*n]),
            Self::ChannelMap(map) => {
                for (dst, on) in params.iter_mut().zip(map) {
                    *dst = u8::from(*on);
                }
                CHANNELS
            }
            Self::VerticalScale(scales) => {
                for (dst, s) in params.iter_mut().zip(scales) {
                    *dst = s.id();
                }
                CHANNELS
            }
            Self::TimeDiv(t) => put(params, &[t.id()]),
            Self::RollRate(r) => put(params, &[r.id()]),
            Self::Mode(m) => put(params, &[*m as u8]),
            Self::ReadyCheck => put(params, &[0x5a]),
            Self::Unknown7 => put(params, &[0x00, 0x00]),
            Self::AcConfig(blob) => put(params, blob),
            Self::TriggerLevel(level) => put(params, &(level & 0x0fff).to_be_bytes()),
            Self::TriggerConfig { channel, slope } => put(params, &[*channel, *slope as u8]),
            Self::BurstLength(b) | Self::BurstChunk(b) => put(params, &[*b as u8]),
            Self::UnknownE4(x) | Self::UnknownE6(x) | Self::GeneratorControl(x) => {
                put(params, &[*x])
            }
            Self::GeneratorOutput(on) => put(params, &[0x08, u8::from(*on)]),
            Self::GeneratorPulseLength(len) => {
                params[0] = 0x01;
                1 + put(&mut params[1..], &len.to_le_bytes())
            }
            Self::GeneratorWaveformLength(len) => put(params, &len.to_le_bytes()),
            Self::GeneratorWaveform { page, data } => {
                params[0] = *page;
                1 + put(&mut params[1..], data)
            }
            Self::Raw(r) => put(params, r.params()),
        };
        1 + n
    }

    /// Total reply length in bytes, including the echo byte if there is one.
    pub fn reply_len(&self) -> usize {
        match self {
            Self::FirmwareVersion | Self::CalTemperature => 2,
            Self::InitBlob(b) => b.reply_len(),
            Self::ReadyCheck | Self::Unknown7 => 2,
            Self::BurstLength(_) | Self::RollReady => 2,
            Self::BurstChunk(_) | Self::RollChunk => PACKET_SIZE,
            Self::UnknownE6(_) => 10,
            Self::Raw(r) => usize::from(r.reply_len),
            _ => 1,
        }
    }

    /// Whether the reply starts with an echo of [`Command::id`].
    ///
    /// Raw data reads (`E5`, `E6`, `E9`, init blobs, burst/roll lengths and
    /// chunks) return bare bytes; hantek1008py reads `E9` without echo too.
    pub fn has_echo(&self) -> bool {
        match self {
            Self::FirmwareVersion
            | Self::CalTemperature
            | Self::InitBlob(_)
            | Self::BurstLength(_)
            | Self::BurstChunk(_)
            | Self::RollReady
            | Self::RollChunk
            | Self::UnknownE6(_) => false,
            Self::Raw(r) => r.echo,
            _ => true,
        }
    }

    /// Delay hantek1008py inserts before writing this request (2 ms for
    /// everything except the keep-alive ping).
    pub fn pre_write_delay(&self) -> Duration {
        match self {
            Self::Ping => Duration::ZERO,
            _ => Duration::from_millis(2),
        }
    }

    /// Delay between writing the request and reading the reply. The device
    /// is slow to answer `A2`, `F5` and `F6` (~0.21 s), `B5` (~19 ms) and
    /// `A4` (~15 ms).
    pub fn reply_delay(&self) -> Duration {
        match self {
            Self::VerticalScale(_) | Self::InitStep5 | Self::InitStep6 => SLOW_REPLY,
            Self::InitBlob(InitBlob::B5) => Duration::from_micros(19_300),
            Self::Mode(_) => Duration::from_millis(15),
            _ => Duration::ZERO,
        }
    }

    /// Whether a longer transport timeout is warranted (init/reset steps and
    /// the slow-to-answer commands).
    pub fn is_slow(&self) -> bool {
        matches!(
            self,
            Self::Init | Self::InitStep5 | Self::InitStep6 | Self::VerticalScale(_)
        )
    }
}

fn put(dst: &mut [u8], src: &[u8]) -> usize {
    dst[..src.len()].copy_from_slice(src);
    src.len()
}

/// Validate a reply's length and echo byte and return the payload.
pub fn check_reply<'a>(cmd: &Command, reply: &'a [u8]) -> Result<&'a [u8], ProtoError> {
    let expected = cmd.reply_len();
    if reply.len() != expected {
        return Err(ProtoError::ReplyLength {
            cmd: cmd.id(),
            expected,
            got: reply.len(),
        });
    }
    if !cmd.has_echo() {
        return Ok(reply);
    }
    match reply.split_first() {
        Some((&echo, payload)) if echo == cmd.id() => Ok(payload),
        Some((&got, _)) => Err(ProtoError::EchoMismatch { cmd: cmd.id(), got }),
        None => Err(ProtoError::ReplyLength {
            cmd: cmd.id(),
            expected,
            got: 0,
        }),
    }
}

/// Parse the 2-byte big-endian byte count returned by `C6` and `C7`.
pub fn parse_byte_count(payload: &[u8]) -> Result<usize, ProtoError> {
    match payload {
        [hi, lo] => Ok(usize::from(u16::from_be_bytes([*hi, *lo]))),
        _ => Err(ProtoError::ReplyLength {
            cmd: id::BURST_LENGTH,
            expected: 2,
            got: payload.len(),
        }),
    }
}

/// Number of 64-byte chunk reads needed to fetch `byte_count` bytes.
pub fn chunk_count(byte_count: usize) -> usize {
    byte_count.div_ceil(PACKET_SIZE)
}

/// Interpret the `A5 5A` payload: `Ok(true)` when a capture is ready (2 or 3),
/// `Ok(false)` while waiting (0 or 1).
pub fn parse_ready(payload: &[u8]) -> Result<bool, ProtoError> {
    match payload {
        [0 | 1] => Ok(false),
        [2 | 3] => Ok(true),
        [value] => Err(ProtoError::UnexpectedValue {
            cmd: id::READY_CHECK,
            value: *value,
        }),
        _ => Err(ProtoError::ReplyLength {
            cmd: id::READY_CHECK,
            expected: 2,
            got: payload.len() + 1,
        }),
    }
}

/// Check the `A7 00 00` payload, which must be `00`.
pub fn parse_unknown7(payload: &[u8]) -> Result<(), ProtoError> {
    match payload {
        [0] => Ok(()),
        [value] => Err(ProtoError::UnexpectedValue {
            cmd: id::UNKNOWN_7,
            value: *value,
        }),
        _ => Err(ProtoError::ReplyLength {
            cmd: id::UNKNOWN_7,
            expected: 2,
            got: payload.len() + 1,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn enc(cmd: Command) -> Vec<u8> {
        let mut buf = [0u8; PACKET_SIZE];
        let n = cmd.encode(&mut buf);
        buf[..n].to_vec()
    }

    #[test]
    fn golden_requests() {
        let t17 = TimeDivId::new(17).unwrap();
        let r440 = RollRateId::new(0x18).unwrap();
        let cases: &[(Command, &[u8])] = &[
            (Command::Init, &[0xb0]),
            (Command::FirmwareVersion, &[0xe9]),
            (Command::CalTemperature, &[0xe5]),
            (Command::InitBlob(InitBlob::B5), &[0xb5]),
            (Command::InitBlob(InitBlob::B6), &[0xb6]),
            (Command::InitBlob(InitBlob::F7), &[0xf7]),
            (Command::InitBlob(InitBlob::F8), &[0xf8]),
            (Command::InitBlob(InitBlob::Fa), &[0xfa]),
            (Command::InitStep5, &[0xf5]),
            (Command::InitStep6, &[0xf6]),
            (Command::Ping, &[0xf3]),
            (Command::Close, &[0xf4]),
            (Command::ChannelCount(8), &[0xa0, 0x08]),
            (
                Command::ChannelMap([true, false, true, false, false, false, false, true]),
                &[0xaa, 1, 0, 1, 0, 0, 0, 0, 1],
            ),
            (
                Command::VerticalScale([
                    ScaleId::V1,
                    ScaleId::V0_125,
                    ScaleId::V0_02,
                    ScaleId::V1,
                    ScaleId::V1,
                    ScaleId::V1,
                    ScaleId::V1,
                    ScaleId::V1,
                ]),
                &[0xa2, 3, 2, 1, 3, 3, 3, 3, 3],
            ),
            (Command::TimeDiv(t17), &[0xa3, 0x11]),
            (Command::RollRate(r440), &[0xa3, 0x18]),
            (Command::Mode(Mode::Burst), &[0xa4, 0x01]),
            (Command::Mode(Mode::Roll), &[0xa4, 0x02]),
            (Command::ReadyCheck, &[0xa5, 0x5a]),
            (Command::Unknown7, &[0xa7, 0x00, 0x00]),
            (
                Command::AcConfig(AC_INIT_1),
                &[0xac, 0x01, 0xf4, 0x00, 0x09, 0xc5, 0x00, 0x09, 0xc5],
            ),
            (
                Command::AcConfig(AC_CONFIGURE),
                &[0xac, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x05, 0x79],
            ),
            (Command::TriggerLevel(2048), &[0xab, 0x08, 0x00]),
            (Command::TriggerLevel(0x0abc), &[0xab, 0x0a, 0xbc]),
            (Command::TriggerLevel(0xffff), &[0xab, 0x0f, 0xff]),
            (
                Command::TriggerConfig {
                    channel: 0,
                    slope: Slope::Rising,
                },
                &[0xc1, 0x00, 0x00],
            ),
            (
                Command::TriggerConfig {
                    channel: 7,
                    slope: Slope::Falling,
                },
                &[0xc1, 0x07, 0x01],
            ),
            (Command::Arm, &[0xc0]),
            (Command::ForceTrigger, &[0xc2]),
            (Command::BurstLength(Block::Two), &[0xc6, 0x02]),
            (Command::BurstLength(Block::Three), &[0xc6, 0x03]),
            (Command::BurstChunk(Block::Two), &[0xa6, 0x02]),
            (Command::BurstChunk(Block::Three), &[0xa6, 0x03]),
            (Command::RollReady, &[0xc7]),
            (Command::RollChunk, &[0xc8]),
            (Command::UnknownE4(1), &[0xe4, 0x01]),
            (Command::UnknownE6(1), &[0xe6, 0x01]),
            (Command::GeneratorControl(0), &[0xb7, 0x00]),
            (Command::GeneratorOutput(false), &[0xbb, 0x08, 0x00]),
            (Command::GeneratorOutput(true), &[0xbb, 0x08, 0x01]),
            (
                Command::GeneratorPulseLength(DEFAULT_GENERATOR_PULSE_LENGTH),
                &[0xb9, 0x01, 0xb0, 0x04, 0x00, 0x00],
            ),
            (Command::GeneratorWaveformLength(4), &[0xbf, 0x04, 0x00]),
        ];
        for (cmd, expected) in cases {
            assert_eq!(enc(*cmd), *expected, "{cmd:?}");
            assert_eq!(cmd.id(), expected[0], "{cmd:?}");
        }
    }

    #[test]
    fn golden_waveform_page() {
        let mut data = [0u8; 62];
        data[..4].copy_from_slice(&[0xf0, 0x0f, 0xf0, 0x0f]);
        let bytes = enc(Command::GeneratorWaveform { page: 1, data });
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[..6], &[0xb8, 0x01, 0xf0, 0x0f, 0xf0, 0x0f]);
        assert!(bytes[6..].iter().all(|&b| b == 0));
    }

    #[test]
    fn raw_command() {
        let raw = RawCommand::new(0xe8, &[0x01, 0x02], 5, true).unwrap();
        let cmd = Command::Raw(raw);
        assert_eq!(enc(cmd), [0xe8, 0x01, 0x02]);
        assert_eq!(cmd.reply_len(), 5);
        assert!(cmd.has_echo());
        assert_eq!(
            RawCommand::new(0, &[0; 64], 1, false),
            Err(ProtoError::ParamsTooLong(64))
        );
        assert_eq!(
            RawCommand::new(0, &[], 0, false),
            Err(ProtoError::ZeroReplyLength)
        );
        let full = RawCommand::new(0x11, &[0x55; 63], 64, false).unwrap();
        assert_eq!(enc(Command::Raw(full)).len(), 64);
    }

    #[test]
    fn reply_shapes() {
        use Command::*;
        let cases: &[(Command, usize, bool)] = &[
            (Init, 1, true),
            (FirmwareVersion, 2, false),
            (CalTemperature, 2, false),
            (InitBlob(super::InitBlob::B5), 64, false),
            (InitBlob(super::InitBlob::B6), 64, false),
            (InitBlob(super::InitBlob::F7), 64, false),
            (InitBlob(super::InitBlob::F8), 64, false),
            (InitBlob(super::InitBlob::Fa), 56, false),
            (InitStep5, 1, true),
            (Ping, 1, true),
            (Close, 1, true),
            (ChannelCount(2), 1, true),
            (ReadyCheck, 2, true),
            (Unknown7, 2, true),
            (BurstLength(Block::Two), 2, false),
            (BurstChunk(Block::Three), 64, false),
            (RollReady, 2, false),
            (RollChunk, 64, false),
            (UnknownE4(1), 1, true),
            (UnknownE6(1), 10, false),
            (GeneratorOutput(true), 1, true),
        ];
        for (cmd, len, echo) in cases {
            assert_eq!(cmd.reply_len(), *len, "{cmd:?}");
            assert_eq!(cmd.has_echo(), *echo, "{cmd:?}");
        }
    }

    #[test]
    fn check_reply_strips_echo() {
        assert_eq!(check_reply(&Command::Ping, &[0xf3]), Ok(&[][..]));
        assert_eq!(
            check_reply(&Command::ReadyCheck, &[0xa5, 0x02]),
            Ok(&[0x02][..])
        );
        assert_eq!(
            check_reply(&Command::FirmwareVersion, &[0x01, 0x09]),
            Ok(&[0x01, 0x09][..])
        );
    }

    #[test]
    fn check_reply_errors() {
        assert_eq!(
            check_reply(&Command::Ping, &[0xf4]),
            Err(ProtoError::EchoMismatch {
                cmd: 0xf3,
                got: 0xf4
            })
        );
        assert_eq!(
            check_reply(&Command::Ping, &[]),
            Err(ProtoError::ReplyLength {
                cmd: 0xf3,
                expected: 1,
                got: 0
            })
        );
        assert_eq!(
            check_reply(&Command::RollReady, &[0, 0, 0]),
            Err(ProtoError::ReplyLength {
                cmd: 0xc7,
                expected: 2,
                got: 3
            })
        );
    }

    #[test]
    fn reply_parsers() {
        assert_eq!(parse_byte_count(&[0x03, 0x00]), Ok(768));
        assert!(parse_byte_count(&[0x03]).is_err());
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(64), 1);
        assert_eq!(chunk_count(65), 2);
        assert_eq!(parse_ready(&[0]), Ok(false));
        assert_eq!(parse_ready(&[1]), Ok(false));
        assert_eq!(parse_ready(&[2]), Ok(true));
        assert_eq!(parse_ready(&[3]), Ok(true));
        assert!(parse_ready(&[4]).is_err());
        assert_eq!(parse_unknown7(&[0]), Ok(()));
        assert!(parse_unknown7(&[1]).is_err());
    }

    #[test]
    fn timing_hints() {
        assert_eq!(Command::Ping.pre_write_delay(), Duration::ZERO);
        assert_eq!(Command::Arm.pre_write_delay(), Duration::from_millis(2));
        assert_eq!(Command::InitStep5.reply_delay(), SLOW_REPLY);
        assert!(Command::Init.is_slow());
        assert!(!Command::Ping.is_slow());
    }
}

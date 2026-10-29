//! Protocol-level error type.

/// Errors raised while building requests or validating replies.
///
/// None of these involve I/O; the transport crate wraps them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoError {
    /// The reply did not have the number of bytes the command produces.
    #[error("reply to 0x{cmd:02x}: expected {expected} bytes, got {got}")]
    ReplyLength {
        /// Command byte.
        cmd: u8,
        /// Expected reply length (including echo byte, if any).
        expected: usize,
        /// Actual reply length.
        got: usize,
    },
    /// The first reply byte did not echo the command byte.
    #[error("reply to 0x{cmd:02x}: echo byte is 0x{got:02x}")]
    EchoMismatch {
        /// Command byte.
        cmd: u8,
        /// First byte actually received.
        got: u8,
    },
    /// A reply carried a value outside the documented set.
    #[error("reply to 0x{cmd:02x}: unexpected value 0x{value:02x}")]
    UnexpectedValue {
        /// Command byte.
        cmd: u8,
        /// Offending value.
        value: u8,
    },
    /// Time/div ID outside 0..=25.
    #[error("time/div id {0} out of range 0..=25")]
    InvalidTimeDivId(u8),
    /// Roll-mode rate ID outside 0x18..=0x24.
    #[error("roll rate id 0x{0:02x} out of range 0x18..=0x24")]
    InvalidRollRateId(u8),
    /// Vertical scale ID outside 1..=3.
    #[error("vertical scale id {0} out of range 1..=3")]
    InvalidScaleId(u8),
    /// Channel index outside 0..=7.
    #[error("channel index {0} out of range 0..=7")]
    InvalidChannel(usize),
    /// Active channel count outside 1..=8.
    #[error("active channel count {0} out of range 1..=8")]
    InvalidChannelCount(usize),
    /// Trigger level does not fit in 12 bits.
    #[error("trigger level {0} exceeds 12 bits (max 4095)")]
    TriggerLevelOutOfRange(u16),
    /// A raw command's parameters do not fit in one 64-byte packet.
    #[error("raw command has {0} parameter bytes; at most 63 fit in one packet")]
    ParamsTooLong(usize),
    /// A raw command was given a zero reply length.
    #[error("reply length must be at least 1")]
    ZeroReplyLength,
    /// Roll-mode byte count is not a whole number of sample rows.
    #[error("roll byte count {len} is not a multiple of the {row}-byte row size")]
    RollLength {
        /// Byte count reported by `0xC7`.
        len: usize,
        /// Bytes per row: (active channels + 1) * 2.
        row: usize,
    },
}

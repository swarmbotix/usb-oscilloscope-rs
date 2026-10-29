//! Transport-level error type.

use nusb::transfer::TransferError;
use usbscope_proto::ProtoError;

/// Errors from opening or talking to the scope.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No device with VID 0x0783 / PID 0x5725 is visible.
    #[error("no Hantek 1008 (USB 0783:5725) found")]
    NotFound,
    /// `open_nth` was given an index past the end of the device list.
    #[error("device index {index} requested but only {count} Hantek 1008 device(s) found")]
    IndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Devices found.
        count: usize,
    },
    /// Enumeration, open or claim failed (permissions, driver binding, ...).
    #[error("USB error: {0}")]
    Usb(#[from] nusb::Error),
    /// OS-level I/O error (e.g. duplicating a file descriptor).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Interface 0 lacks the expected bulk endpoint.
    #[error("interface 0 has no bulk {0} endpoint")]
    MissingEndpoint(&'static str),
    /// A transfer did not complete within the timeout.
    #[error("timed out on USB {direction} for command 0x{cmd:02x}")]
    Timeout {
        /// Command byte of the transaction.
        cmd: u8,
        /// `"write"` or `"read"`.
        direction: &'static str,
    },
    /// A transfer failed (stall, disconnect, ...).
    #[error("USB {direction} for command 0x{cmd:02x} failed: {source}")]
    Transfer {
        /// Command byte of the transaction.
        cmd: u8,
        /// `"write"` or `"read"`.
        direction: &'static str,
        /// Underlying transfer error.
        #[source]
        source: TransferError,
    },
    /// The reply violated the protocol (length, echo, unexpected value).
    #[error(transparent)]
    Proto(#[from] ProtoError),
    /// `A5 5A` never reported a finished capture.
    #[error("capture not ready after {0} ready checks (is the trigger firing?)")]
    NotReady(u32),
    /// `roll_read` was called without `roll_start`.
    #[error("roll mode is not running; call roll_start first")]
    NotRolling,
}

/// Result alias for this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

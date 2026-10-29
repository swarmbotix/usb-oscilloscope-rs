//! USB transport and session layer for the Hantek 1008/1008C oscilloscope.
//!
//! [`Scope`] finds the device (`0783:5725`) with [`nusb`] (pure Rust: Linux
//! usbfs, Windows WinUSB, macOS IOKit, Android via a usbfs fd), claims
//! interface 0, and runs the vendor command sequences from `usbscope-proto`
//! over its bulk OUT/IN pipe pair: init, configuration, burst and roll
//! capture, keep-alive and close.
//!
//! Each request is one bulk write followed by one bulk read of the reply,
//! with the inter-command delays the hardware needs. Every transaction is
//! logged at `trace` level as hex.
//!
//! On Windows the device must be bound to WinUSB (not Hantek's
//! `STM32MS.sys`); on Linux the user needs access to the usbfs node.

#![warn(missing_docs)]

mod error;
mod scope;
mod transport;

pub use error::{Error, Result};
#[cfg(not(target_os = "android"))]
pub use scope::DeviceInfo;
pub use scope::{DEFAULT_TIMEOUT, InitReport, PING_INTERVAL, SLOW_TIMEOUT, Scope, TriggerMode};
pub use transport::{Transport, UsbTransport};
pub use usbscope_proto as proto;

/// Re-exported so [`Transport`] implementors can build error values.
pub use nusb::transfer::TransferError;

/// Lowercase hex with spaces between bytes, e.g. `"a5 5a"`.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

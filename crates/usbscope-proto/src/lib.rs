//! Sans-I/O protocol layer for the Hantek 1008/1008C 8-channel USB
//! oscilloscope (USB `0783:5725`).
//!
//! This crate knows how to encode every request, how long each reply is and
//! whether it echoes the command byte, the lookup tables (time/div, roll rate,
//! vertical scale, sample-rate factors), and how to turn sample bytes into
//! per-channel values and volts. It performs no I/O and has no USB dependency;
//! `usbscope-usb` drives it over a real transport.
//!
//! The command layouts and the init/capture sequences mirror
//! [hantek1008py](https://github.com/mfg92/hantek1008py), the only
//! implementation verified on hardware, cross-checked against the vendor DLL.
//!
//! `#![no_std]` + `alloc`, so it can also be embedded in firmware tooling.

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod cmd;
pub mod config;
pub mod error;
pub mod frame;
pub mod tables;

pub use cmd::{
    Block, Command, InitBlob, Mode, PACKET_SIZE, PRODUCT_ID, RawCommand, Slope, VENDOR_ID,
    check_reply,
};
pub use config::{Config, Trigger};
pub use error::ProtoError;
pub use frame::{ZeroOffsets, decode_u16_le, deinterleave, deinterleave_roll, raw_to_volts};
pub use tables::{CHANNELS, RollRateId, ScaleId, TimeDivId, sample_rate_factor};

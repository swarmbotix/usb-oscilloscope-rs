//! `usbscope`: command-line tool for the Hantek 1008/1008C USB oscilloscope.
//!
//! Desktop only: on Android the device is reached through `UsbManager` and a
//! file descriptor (see `Scope::from_fd`), so the binary is a stub there.

#[cfg(not(target_os = "android"))]
mod cli;
#[cfg(not(target_os = "android"))]
mod csv;
#[cfg(not(target_os = "android"))]
mod parse;

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    cli::run()
}

#[cfg(target_os = "android")]
fn main() {
    eprintln!("usbscope: the CLI is not supported on Android; use the app/FFI layer");
    std::process::exit(1);
}

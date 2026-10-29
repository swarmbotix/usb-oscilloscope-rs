//! Byte-level transport: the [`Transport`] trait and its `nusb` implementation.

use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};
use nusb::{Endpoint, Interface, MaybeFuture};

use crate::error::{Error, Result};

/// One bulk OUT + one bulk IN pipe. [`crate::Scope`] drives the protocol over
/// any implementation, which lets the command sequences be tested against a
/// simulated device.
pub trait Transport {
    /// Send one request packet.
    fn write(&mut self, data: &[u8], timeout: Duration) -> Result<(), TransferError>;

    /// Receive one reply of up to `max_len` bytes (fewer if the device sends
    /// a short packet).
    fn read(&mut self, max_len: usize, timeout: Duration) -> Result<Vec<u8>, TransferError>;

    /// Wait between protocol steps. Simulated transports may skip this.
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    /// Reset the device after closing (re-enumerates it). Best effort; the
    /// default does nothing.
    fn reset(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// [`Transport`] over a claimed `nusb` interface.
pub struct UsbTransport {
    ep_out: Endpoint<Bulk, Out>,
    ep_in: Endpoint<Bulk, In>,
    // Field order matters for drop: endpoints release before the interface,
    // the interface before the device.
    _interface: Interface,
    device: nusb::Device,
}

impl UsbTransport {
    /// Claim interface 0 of an opened device and open its first bulk OUT and
    /// bulk IN endpoints.
    pub fn new(device: nusb::Device) -> Result<Self> {
        ensure_configured(&device);
        // Detaches a kernel driver on Linux; same as claim_interface elsewhere.
        let interface = device.detach_and_claim_interface(0).wait()?;

        let (mut out_addr, mut in_addr) = (None, None);
        if let Some(desc) = interface.descriptor() {
            for ep in desc.endpoints() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::Out if out_addr.is_none() => out_addr = Some(ep.address()),
                    Direction::In if in_addr.is_none() => in_addr = Some(ep.address()),
                    _ => {}
                }
            }
        }
        let out_addr = out_addr.ok_or(Error::MissingEndpoint("OUT"))?;
        let in_addr = in_addr.ok_or(Error::MissingEndpoint("IN"))?;
        log::debug!("bulk endpoints: OUT 0x{out_addr:02x}, IN 0x{in_addr:02x}");

        let ep_out = interface.endpoint::<Bulk, Out>(out_addr)?;
        let ep_in = interface.endpoint::<Bulk, In>(in_addr)?;
        Ok(Self {
            ep_out,
            ep_in,
            _interface: interface,
            device,
        })
    }

    /// Bulk endpoint addresses `(out, in)`.
    pub fn endpoint_addresses(&self) -> (u8, u8) {
        (
            self.ep_out.endpoint_address(),
            self.ep_in.endpoint_address(),
        )
    }
}

/// hantek1008py calls `set_configuration()` on connect. Most OSes configure a
/// single-configuration device automatically, so only do it when unconfigured
/// (and never on Windows, where nusb cannot).
fn ensure_configured(device: &nusb::Device) {
    if device.active_configuration().is_ok() {
        return;
    }
    let Some(value) = device
        .configurations()
        .next()
        .map(|c| c.configuration_value())
    else {
        return;
    };
    if cfg!(target_os = "windows") {
        return;
    }
    if let Err(e) = device.set_configuration(value).wait() {
        log::warn!("set_configuration({value}) failed: {e}");
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, data: &[u8], timeout: Duration) -> Result<(), TransferError> {
        let completion = self.ep_out.transfer_blocking(Buffer::from(data), timeout);
        completion.status?;
        if completion.actual_len != data.len() {
            return Err(TransferError::Fault);
        }
        Ok(())
    }

    fn read(&mut self, max_len: usize, timeout: Duration) -> Result<Vec<u8>, TransferError> {
        // IN transfers must request a whole number of max-size packets; a
        // short packet from the device ends the transfer early.
        let mps = self.ep_in.max_packet_size().max(1);
        let requested = max_len.div_ceil(mps).max(1) * mps;
        let completion = self
            .ep_in
            .transfer_blocking(Buffer::new(requested), timeout);
        match completion.status {
            Ok(()) => Ok(completion.buffer.into_vec()),
            Err(TransferError::Stall) => {
                if let Err(e) = self.ep_in.clear_halt().wait() {
                    log::warn!("clear_halt on IN endpoint failed: {e}");
                }
                Err(TransferError::Stall)
            }
            Err(e) => Err(e),
        }
    }

    fn reset(&mut self) -> Result<(), String> {
        self.device.reset().wait().map_err(|e| e.to_string())
    }
}

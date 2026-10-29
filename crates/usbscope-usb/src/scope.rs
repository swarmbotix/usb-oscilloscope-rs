//! [`Scope`]: protocol sequencing (init, configure, burst/roll capture, close)
//! on top of a [`Transport`].

use std::time::{Duration, Instant};

use nusb::MaybeFuture;
use nusb::transfer::TransferError;
use usbscope_proto::cmd::{
    AC_INIT_1, AC_INIT_2, DEFAULT_GENERATOR_PULSE_LENGTH, chunk_count, parse_byte_count,
    parse_ready, parse_unknown7,
};
use usbscope_proto::frame::mean;
use usbscope_proto::{
    Block, CHANNELS, Command, Config, InitBlob, Mode, PACKET_SIZE, ProtoError, RollRateId, ScaleId,
    Slope, TimeDivId, Trigger, ZeroOffsets, check_reply, decode_u16_le, deinterleave,
    deinterleave_roll,
};

use crate::error::{Error, Result};
use crate::transport::{Transport, UsbTransport};

/// Default per-transfer timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);
/// Timeout for init/reset steps and the slow-to-answer commands.
pub const SLOW_TIMEOUT: Duration = Duration::from_secs(5);

/// The vendor init sends `B0` twice with this gap.
const INIT_GAP: Duration = Duration::from_millis(700);
/// Pause between `C0` and `C2` during the init's zero-offset captures.
const ARM_TO_FORCE: Duration = Duration::from_micros(12_400);
/// Pause between unsuccessful ready checks (followed by a ping).
const READY_POLL_GAP: Duration = Duration::from_millis(20);
/// Ready checks before giving up.
const READY_ATTEMPTS: u32 = 20;
/// Keep-alive ping period while idle.
pub const PING_INTERVAL: Duration = Duration::from_millis(10);
/// How long `close` waits for a stale reply left in the IN pipe.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

/// A Hantek 1008 visible on the bus.
///
/// Not available on Android, where enumeration goes through `UsbManager`;
/// use `Scope::from_fd` (Linux/Android only) there.
#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Bus / host controller identifier (unavailable on Android).
    pub bus: Option<String>,
    /// Device address on the bus (unavailable on Android).
    pub address: Option<u8>,
    /// Serial number string descriptor.
    pub serial: Option<String>,
    /// Manufacturer string descriptor.
    pub manufacturer: Option<String>,
    /// Product string descriptor.
    pub product: Option<String>,
    /// Bound kernel driver (Windows only; should be `WinUSB` for nusb).
    pub driver: Option<String>,
    info: nusb::DeviceInfo,
}

#[cfg(not(target_os = "android"))]
impl DeviceInfo {
    fn from_nusb(info: nusb::DeviceInfo) -> Self {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let (bus, address) = (Some(info.bus_id().to_owned()), Some(info.device_address()));
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let (bus, address) = (None, None);

        #[cfg(target_os = "windows")]
        let driver = info.driver().map(str::to_owned);
        #[cfg(not(target_os = "windows"))]
        let driver = None;

        // Cached strings first; fall back to asking the device, best effort
        // (fails harmlessly when another driver owns it).
        let manufacturer = info
            .manufacturer_string()
            .map(str::to_owned)
            .or_else(|| info.get_manufacturer_string().wait().ok().flatten());
        let product = info
            .product_string()
            .map(str::to_owned)
            .or_else(|| info.get_product_string().wait().ok().flatten());
        let serial = info
            .serial_number()
            .map(str::to_owned)
            .or_else(|| info.get_serial_number_string().wait().ok().flatten());

        Self {
            bus,
            address,
            serial,
            manufacturer,
            product,
            driver,
            info,
        }
    }

    /// Open this device.
    pub fn open(&self) -> Result<Scope> {
        let device = self.info.open().wait()?;
        Scope::from_device(device)
    }
}

/// How a burst capture is triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    /// Send `C2` right after arming (hantek1008py behaviour): returns
    /// immediately regardless of the input signal.
    Force,
    /// Do not force; poll the ready flag for up to this long, waiting for
    /// the configured trigger condition.
    Wait(Duration),
}

/// What the init sequence read back from the device.
#[derive(Debug, Clone, PartialEq)]
pub struct InitReport {
    /// `E9` reply, 2 raw bytes (hantek1008py sees `01 09`; not asserted).
    pub firmware_version: [u8; 2],
    /// `E5` reply, 2 raw bytes (little-endian count, unit unknown).
    pub cal_temperature: [u8; 2],
    /// Init blobs (`B5`, `B6`, `F7`, `F8`, `FA`) from the first read.
    pub blobs: Vec<(InitBlob, Vec<u8>)>,
    /// `E6 01` reply from the last init phase (10 raw bytes).
    pub e6: Vec<u8>,
    /// Zero offsets measured with every vertical scale.
    pub zero_offsets: ZeroOffsets,
}

/// An open Hantek 1008 session.
///
/// Typical use: [`Scope::open`], [`Scope::init_with`], then
/// [`Scope::capture_burst`] or the roll-mode calls, and finally
/// [`Scope::close`].
pub struct Scope<T: Transport = UsbTransport> {
    transport: T,
    timeout: Duration,
    slow_timeout: Duration,
    config: Config,
    zero_offsets: ZeroOffsets,
    rolling: bool,
}

impl Scope<UsbTransport> {
    /// Enumerate attached Hantek 1008 devices.
    #[cfg(not(target_os = "android"))]
    pub fn list() -> Result<Vec<DeviceInfo>> {
        Ok(nusb::list_devices()
            .wait()?
            .filter(|d| {
                d.vendor_id() == usbscope_proto::VENDOR_ID
                    && d.product_id() == usbscope_proto::PRODUCT_ID
            })
            .map(DeviceInfo::from_nusb)
            .collect())
    }

    /// Open the first Hantek 1008 found.
    #[cfg(not(target_os = "android"))]
    pub fn open() -> Result<Self> {
        Self::open_nth(0)
    }

    /// Open the `index`-th Hantek 1008 in [`Scope::list`] order.
    #[cfg(not(target_os = "android"))]
    pub fn open_nth(index: usize) -> Result<Self> {
        let devices = Self::list()?;
        match devices.get(index) {
            Some(d) => d.open(),
            None if devices.is_empty() => Err(Error::NotFound),
            None => Err(Error::IndexOutOfRange {
                index,
                count: devices.len(),
            }),
        }
    }

    /// Open from a usbfs file descriptor **without taking ownership of it**,
    /// e.g. Android's `UsbDeviceConnection.getFileDescriptor()`.
    ///
    /// Ownership: the descriptor is duplicated (`dup`) and the [`Scope`]
    /// owns and closes only its duplicate. The caller still owns `fd` and
    /// remains responsible for closing it (on Android: keep the
    /// `UsbDeviceConnection` open while the scope is in use and call
    /// `connection.close()` afterwards; Rust never closes the original).
    /// To hand ownership over instead, use [`Scope::from_owned_fd`].
    ///
    /// # Safety
    /// `fd` must be an open file descriptor for the duration of this call.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub unsafe fn from_fd(fd: std::os::fd::RawFd) -> Result<Self> {
        // SAFETY: the caller guarantees `fd` is open for this call; it is
        // only borrowed long enough to duplicate it.
        let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
        Self::from_owned_fd(borrowed.try_clone_to_owned()?)
    }

    /// Open from a usbfs file descriptor, **taking ownership**: it is closed
    /// when the [`Scope`] (and its transport) is dropped. On Android, obtain
    /// such a descriptor with `ParcelFileDescriptor.detachFd()` or a `dup`,
    /// never the raw `getFileDescriptor()` value the connection still owns.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn from_owned_fd(fd: std::os::fd::OwnedFd) -> Result<Self> {
        let device = nusb::Device::from_fd(fd).wait()?;
        Self::from_device(device)
    }

    /// Wrap an already-opened `nusb` device.
    pub fn from_device(device: nusb::Device) -> Result<Self> {
        Ok(Self::with_transport(UsbTransport::new(device)?))
    }
}

impl<T: Transport> Scope<T> {
    /// Drive the protocol over a custom transport.
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport,
            timeout: DEFAULT_TIMEOUT,
            slow_timeout: SLOW_TIMEOUT,
            config: Config::default(),
            zero_offsets: ZeroOffsets::default(),
            rolling: false,
        }
    }

    /// Set the per-transfer timeout. Slow commands use at least
    /// [`SLOW_TIMEOUT`].
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        self.slow_timeout = timeout.max(SLOW_TIMEOUT);
    }

    /// Current per-transfer timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Configuration last sent to the device.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Zero offsets measured by the last init (nominal 2048 before that).
    pub fn zero_offsets(&self) -> &ZeroOffsets {
        &self.zero_offsets
    }

    /// Underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Write one request and read its raw reply without validating it.
    pub fn transact_raw(&mut self, cmd: &Command) -> Result<Vec<u8>> {
        self.exchange(cmd, cmd.pre_write_delay(), cmd.reply_delay())
    }

    /// Write one request, read the reply, validate length and echo, and
    /// return the payload (reply minus echo byte).
    pub fn transact(&mut self, cmd: &Command) -> Result<Vec<u8>> {
        let reply = self.transact_raw(cmd)?;
        Ok(check_reply(cmd, &reply)?.to_vec())
    }

    fn transact_timed(
        &mut self,
        cmd: &Command,
        pre_write: Duration,
        reply_delay: Duration,
    ) -> Result<Vec<u8>> {
        let reply = self.exchange(cmd, pre_write, reply_delay)?;
        Ok(check_reply(cmd, &reply)?.to_vec())
    }

    fn exchange(
        &mut self,
        cmd: &Command,
        pre_write: Duration,
        reply_delay: Duration,
    ) -> Result<Vec<u8>> {
        let mut packet = [0u8; PACKET_SIZE];
        let len = cmd.encode(&mut packet);
        let request = &packet[..len];
        let timeout = if cmd.is_slow() {
            self.slow_timeout
        } else {
            self.timeout
        };
        let id = cmd.id();

        self.pause(pre_write);
        log::trace!(">[{len:2}] {}", crate::hex(request));
        self.transport
            .write(request, timeout)
            .map_err(|e| transfer_error(id, "write", e))?;
        self.pause(reply_delay);
        let reply = self
            .transport
            .read(cmd.reply_len(), timeout)
            .map_err(|e| transfer_error(id, "read", e))?;
        log::trace!("<[{:2}] {}", reply.len(), crate::hex(&reply));
        Ok(reply)
    }

    fn pause(&mut self, duration: Duration) {
        if !duration.is_zero() {
            self.transport.sleep(duration);
        }
    }

    /// Send one keep-alive `F3`.
    pub fn ping(&mut self) -> Result<()> {
        self.transact(&Command::Ping).map(drop)
    }

    /// Ping every [`PING_INTERVAL`] for `duration`, as the vendor software
    /// does whenever it is idle.
    pub fn keep_alive(&mut self, duration: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < duration {
            self.ping()?;
            self.transport.sleep(PING_INTERVAL);
        }
        Ok(())
    }

    /// Run the vendor init with default settings ([`Config::default`]).
    pub fn init(&mut self) -> Result<InitReport> {
        self.init_with(&Config::default())
    }

    /// Run the full vendor init sequence, measure zero offsets, and apply
    /// `config`. Mirrors hantek1008py's `init()` step for step.
    pub fn init_with(&mut self, config: &Config) -> Result<InitReport> {
        config.validate()?;
        log::debug!("init phase 1: reset, blobs, defaults");
        let (cal_temperature, blobs) = self.init_phase1()?;
        log::debug!("init phase 2: zero offsets");
        let zero_offsets = self.measure_zero_offsets()?;
        self.zero_offsets = zero_offsets;
        log::debug!("init phase 3: final config");
        let e6 = self.init_phase3(config, &blobs)?;
        let firmware_version = self.firmware_version()?;
        Ok(InitReport {
            firmware_version,
            cal_temperature,
            blobs,
            e6,
            zero_offsets,
        })
    }

    #[allow(clippy::type_complexity)]
    fn init_phase1(&mut self) -> Result<([u8; 2], Vec<(InitBlob, Vec<u8>)>)> {
        self.transact(&Command::Init)?;
        self.transport.sleep(INIT_GAP);
        self.transact(&Command::Init)?;
        self.ping()?;

        // Generator idle: pulse length 1200, control 0, output off.
        self.transact(&Command::GeneratorPulseLength(
            DEFAULT_GENERATOR_PULSE_LENGTH,
        ))?;
        self.transact(&Command::GeneratorControl(0))?;
        self.transact(&Command::GeneratorOutput(false))?;

        let mut blobs = Vec::with_capacity(InitBlob::ALL.len());
        let mut cal_temperature = [0u8; 2];
        // Vendor order: B5, B6, E5, F7, F8, FA.
        for blob in [InitBlob::B5, InitBlob::B6] {
            blobs.push((blob, self.transact(&Command::InitBlob(blob))?));
        }
        cal_temperature.copy_from_slice(&self.transact(&Command::CalTemperature)?);
        for blob in [InitBlob::F7, InitBlob::F8, InitBlob::Fa] {
            blobs.push((blob, self.transact(&Command::InitBlob(blob))?));
        }

        self.transact(&Command::InitStep5)?;

        // All channels on, 500 us/div, trigger CH1 rising.
        self.transact(&Command::ChannelCount(CHANNELS as u8))?;
        self.transact(&Command::ChannelMap([true; CHANNELS]))?;
        self.transact(&Command::TimeDiv(TimeDivId::DEFAULT))?;
        self.transact(&Command::TriggerConfig {
            channel: 0,
            slope: Slope::Rising,
        })?;
        parse_unknown7(&self.transact(&Command::Unknown7)?)?;
        self.transact(&Command::AcConfig(AC_INIT_1))?;
        Ok((cal_temperature, blobs))
    }

    /// Capture once per vertical scale with all channels on and average each
    /// channel. Assumes the inputs are idle.
    fn measure_zero_offsets(&mut self) -> Result<ZeroOffsets> {
        let mut offsets = ZeroOffsets::default();
        for scale in ScaleId::ALL {
            self.ping()?;
            self.transact(&Command::VerticalScale([scale; CHANNELS]))?;
            let mode = Command::Mode(Mode::Burst);
            self.transact_timed(&mode, mode.pre_write_delay(), Duration::ZERO)?;
            self.transact(&Command::Arm)?;
            self.transport.sleep(ARM_TO_FORCE);
            self.transact(&Command::ForceTrigger)?;
            self.wait_ready(READY_ATTEMPTS)?;
            let samples = self.read_burst_samples()?;
            for (ch, values) in deinterleave(&samples, CHANNELS).iter().enumerate() {
                if let Some(avg) = mean(values) {
                    offsets.set(scale, ch, avg);
                }
            }
            log::debug!(
                "zero offsets @ scale {}: {:?}",
                scale.volts_per_unit(),
                offsets.for_scale(scale)
            );
        }
        Ok(offsets)
    }

    fn init_phase3(
        &mut self,
        config: &Config,
        first_blobs: &[(InitBlob, Vec<u8>)],
    ) -> Result<Vec<u8>> {
        self.transact(&Command::InitStep6)?;
        // The vendor re-reads E5 and the F7/F8/FA blobs here; hantek1008py
        // asserts device-specific values, we only log a mismatch.
        self.transact(&Command::CalTemperature)?;
        for blob in [InitBlob::F7, InitBlob::F8, InitBlob::Fa] {
            let again = self.transact(&Command::InitBlob(blob))?;
            let first = first_blobs.iter().find(|(b, _)| *b == blob);
            if first.is_some_and(|(_, data)| *data != again) {
                log::warn!("init blob 0x{:02x} changed between reads", blob.id());
            }
        }
        self.transact(&Command::TimeDiv(config.time_div))?;
        self.transact(&Command::AcConfig(AC_INIT_2))?;
        self.transact(&Command::UnknownE4(1))?;
        let e6 = self.transact(&Command::UnknownE6(1))?;
        self.configure(config)?;
        Ok(e6)
    }

    /// Read the firmware version (`E9`, 2 raw bytes).
    pub fn firmware_version(&mut self) -> Result<[u8; 2]> {
        let payload = self.transact(&Command::FirmwareVersion)?;
        let mut version = [0u8; 2];
        version.copy_from_slice(&payload);
        Ok(version)
    }

    /// Apply channel map, scales, time/div and trigger (ping, then `A0`,
    /// `AA`, `A2`, `A3`, `C1`, `A7`, `AC`, `AB`).
    pub fn configure(&mut self, config: &Config) -> Result<()> {
        config.validate()?;
        self.ping()?;
        for cmd in config.commands() {
            let payload = self.transact(&cmd)?;
            if cmd == Command::Unknown7 {
                parse_unknown7(&payload)?;
            }
        }
        self.config = *config;
        Ok(())
    }

    /// Change only the trigger (re-sends the full configuration).
    pub fn set_trigger(&mut self, trigger: Trigger) -> Result<()> {
        let config = Config {
            trigger,
            ..self.config
        };
        self.configure(&config)
    }

    /// Capture one burst and return raw 12-bit samples per active channel
    /// (ascending channel order, as in [`Config::active_channels`]).
    ///
    /// The capture is armed and immediately force-triggered (`C0` then
    /// `C2`), exactly like hantek1008py. Use [`Scope::capture_burst_with`]
    /// and [`TriggerMode::Wait`] to wait for the configured trigger instead.
    pub fn capture_burst(&mut self) -> Result<Vec<Vec<u16>>> {
        self.capture_burst_with(TriggerMode::Force)
    }

    /// Capture one burst with an explicit [`TriggerMode`].
    pub fn capture_burst_with(&mut self, trigger: TriggerMode) -> Result<Vec<Vec<u16>>> {
        self.ping()?;
        // hantek1008py notes E4/E6 around a capture are "not necessarily
        // required"; kept to match the vendor traffic.
        self.transact(&Command::UnknownE4(1))?;
        self.transact(&Command::UnknownE6(1))?;
        self.transact(&Command::Mode(Mode::Burst))?;
        self.transact(&Command::Arm)?;
        let attempts = match trigger {
            TriggerMode::Force => {
                self.transact(&Command::ForceTrigger)?;
                READY_ATTEMPTS
            }
            // Unverified on hardware: hantek1008py always forces.
            TriggerMode::Wait(max_wait) => {
                let polls = max_wait.as_millis().div_ceil(READY_POLL_GAP.as_millis());
                u32::try_from(polls).unwrap_or(u32::MAX).max(1)
            }
        };
        self.wait_ready(attempts)?;
        let samples = self.read_burst_samples()?;
        self.transact(&Command::UnknownE4(1))?;
        self.transact(&Command::UnknownE6(1))?;
        Ok(deinterleave(&samples, self.config.active_count()))
    }

    /// Poll `A5 5A` until the device reports a finished capture.
    fn wait_ready(&mut self, attempts: u32) -> Result<()> {
        for attempt in 1..=attempts {
            if parse_ready(&self.transact(&Command::ReadyCheck)?)? {
                log::debug!("capture ready after {attempt} check(s)");
                return Ok(());
            }
            self.transport.sleep(READY_POLL_GAP);
            self.ping()?;
        }
        Err(Error::NotReady(attempts))
    }

    /// Read burst blocks 2 and 3 and decode them into interleaved samples.
    fn read_burst_samples(&mut self) -> Result<Vec<u16>> {
        let mut bytes = Vec::new();
        for block in Block::ALL {
            bytes.extend(self.read_block(block)?);
        }
        Ok(decode_u16_le(&bytes))
    }

    fn read_block(&mut self, block: Block) -> Result<Vec<u8>> {
        let len = parse_byte_count(&self.transact(&Command::BurstLength(block))?)?;
        let mut data = Vec::with_capacity(chunk_count(len) * PACKET_SIZE);
        for _ in 0..chunk_count(len) {
            data.extend(self.transact(&Command::BurstChunk(block))?);
        }
        data.truncate(len);
        log::debug!("burst block {}: {len} bytes", block as u8);
        Ok(data)
    }

    /// Switch to roll mode at `rate` and start streaming.
    pub fn roll_start(&mut self, rate: RollRateId) -> Result<()> {
        self.transact(&Command::RollRate(rate))?;
        self.transport.sleep(PING_INTERVAL);
        self.ping()?;
        let mode = Command::Mode(Mode::Roll);
        self.transact_timed(&mode, mode.pre_write_delay(), Duration::ZERO)?;
        self.transact(&Command::Arm)?;
        self.transact(&Command::ForceTrigger)?;
        self.rolling = true;
        Ok(())
    }

    /// Block until roll-mode data is available, then return it per active
    /// channel. The extra 9th value in each roll-mode frame is dropped.
    ///
    /// Polls (ping + `C7`) back to back with no upper bound, like
    /// hantek1008py; at the slowest rate (1/16 Hz) this can take 16 s.
    pub fn roll_read(&mut self) -> Result<Vec<Vec<u16>>> {
        if !self.rolling {
            return Err(Error::NotRolling);
        }
        let channels = self.config.active_count();
        let row = (channels + 1) * 2;
        let len = loop {
            self.ping()?;
            let len = parse_byte_count(&self.transact(&Command::RollReady)?)?;
            if len > 0 {
                break len;
            }
        };
        if len % row != 0 {
            return Err(ProtoError::RollLength { len, row }.into());
        }
        let mut bytes = Vec::with_capacity(len);
        let mut remaining = len;
        while remaining > 0 {
            let chunk = self.transact(&Command::RollChunk)?;
            let take = remaining.min(chunk.len());
            bytes.extend_from_slice(&chunk[..take]);
            remaining -= take;
        }
        Ok(deinterleave_roll(&decode_u16_le(&bytes), channels))
    }

    /// Leave roll mode bookkeeping. No vendor "stop" command is known; this
    /// discards any stale reply and pings. Call [`Scope::configure`] before
    /// the next burst capture to restore the time/div.
    pub fn roll_stop(&mut self) -> Result<()> {
        self.rolling = false;
        self.drain();
        self.ping()
    }

    /// Read and discard a reply left in the IN pipe by an interrupted
    /// transaction (100 ms timeout, errors ignored).
    pub fn drain(&mut self) {
        match self.transport.read(PACKET_SIZE, DRAIN_TIMEOUT) {
            Ok(data) if !data.is_empty() => {
                log::debug!("drained leftover reply: {}", crate::hex(&data))
            }
            _ => log::trace!("no leftover data"),
        }
    }

    /// End the session: drain, ping, `F4`, then reset the device (best effort;
    /// unsupported on Windows).
    pub fn close(mut self) -> Result<()> {
        self.drain();
        self.ping()?;
        self.transact(&Command::Close)?;
        if let Err(e) = self.transport.reset() {
            log::debug!("device reset after close not performed: {e}");
        }
        Ok(())
    }
}

fn transfer_error(cmd: u8, direction: &'static str, source: TransferError) -> Error {
    match source {
        TransferError::Cancelled => Error::Timeout { cmd, direction },
        source => Error::Transfer {
            cmd,
            direction,
            source,
        },
    }
}

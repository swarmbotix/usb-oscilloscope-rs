//! Drives `Scope` against a simulated device and checks the exact request
//! sequence against hantek1008py's, plus decoding of the simulated data.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use usbscope_usb::proto::{CHANNELS, Config, RollRateId, ScaleId, Slope, TimeDivId, Trigger};
use usbscope_usb::{Error, Scope, TransferError, Transport, TriggerMode};

/// Answers every known command plausibly and records all requests.
#[derive(Default)]
struct FakeDevice {
    log: Vec<Vec<u8>>,
    pending: Option<Vec<u8>>,
    /// Interleaved burst samples served from blocks 2 and 3 (split in half).
    burst: Vec<u16>,
    block_offset: [usize; 2],
    /// `A5` answers before reporting ready.
    not_ready_polls: u32,
    polls_left: u32,
    /// Roll payloads served by successive non-empty `C7` replies.
    roll: VecDeque<Vec<u8>>,
    roll_current: Vec<u8>,
    /// Override the echo byte of the next reply.
    corrupt_echo: Option<u8>,
    sleeps: Vec<Duration>,
    /// Copy of the request log that outlives the transport.
    shared_log: Rc<RefCell<Vec<String>>>,
}

impl FakeDevice {
    fn with_burst(burst: Vec<u16>) -> Self {
        Self {
            burst,
            ..Self::default()
        }
    }

    fn block_bytes(&self, block: u8) -> Vec<u8> {
        let half = self.burst.len() / 2;
        let samples = if block == 2 {
            &self.burst[..half]
        } else {
            &self.burst[half..]
        };
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn respond(&mut self, req: &[u8]) -> Vec<u8> {
        let cmd = req[0];
        match cmd {
            0xe9 => vec![0x01, 0x09],
            0xe5 => vec![0xd6, 0x06],
            0xb5 | 0xb6 | 0xf7 | 0xf8 => vec![cmd; 64],
            0xfa => vec![cmd; 56],
            0xe6 => vec![0xe9, 0x06, 0xe5, 0x06, 0xe4, 0x06, 0xe4, 0x06, 0xe5, 0x06],
            0xa7 => vec![0xa7, 0x00],
            0xa5 => {
                if self.polls_left > 0 {
                    self.polls_left -= 1;
                    vec![0xa5, 0x01]
                } else {
                    self.polls_left = self.not_ready_polls;
                    vec![0xa5, 0x02]
                }
            }
            0xc6 => {
                let block = req[1];
                self.block_offset[usize::from(block - 2)] = 0;
                let len = self.block_bytes(block).len() as u16;
                len.to_be_bytes().to_vec()
            }
            0xa6 => {
                let block = req[1];
                let bytes = self.block_bytes(block);
                let off = &mut self.block_offset[usize::from(block - 2)];
                let mut chunk = vec![0u8; 64];
                let end = (*off + 64).min(bytes.len());
                chunk[..end - *off].copy_from_slice(&bytes[*off..end]);
                *off = end;
                chunk
            }
            0xc7 => {
                if self.roll_current.is_empty() {
                    self.roll_current = self.roll.pop_front().unwrap_or_default();
                }
                (self.roll_current.len() as u16).to_be_bytes().to_vec()
            }
            0xc8 => {
                let take = self.roll_current.len().min(64);
                let mut chunk: Vec<u8> = self.roll_current.drain(..take).collect();
                chunk.resize(64, 0);
                chunk
            }
            _ => vec![self.corrupt_echo.take().unwrap_or(cmd)],
        }
    }

    fn requests(&self) -> Vec<String> {
        self.log.iter().map(|r| usbscope_usb::hex(r)).collect()
    }
}

impl Transport for FakeDevice {
    fn write(&mut self, data: &[u8], _timeout: Duration) -> Result<(), TransferError> {
        self.log.push(data.to_vec());
        self.shared_log.borrow_mut().push(usbscope_usb::hex(data));
        self.pending = Some(self.respond(data));
        Ok(())
    }

    fn read(&mut self, _max_len: usize, _timeout: Duration) -> Result<Vec<u8>, TransferError> {
        // Nothing pending behaves like a timeout.
        self.pending.take().ok_or(TransferError::Cancelled)
    }

    fn sleep(&mut self, duration: Duration) {
        self.sleeps.push(duration);
    }
}

/// 8-channel burst where channel `c` always reads `base + c`; 40 frames.
fn constant_burst(base: u16) -> Vec<u16> {
    (0..40)
        .flat_map(|_| (0..CHANNELS as u16).map(move |c| base + c))
        .collect()
}

fn burst_reads(lines: &mut Vec<&'static str>) {
    // 40 frames * 8 ch * 2 B = 640 B, 320 B per block = 5 chunks each.
    lines.extend(["c6 02", "a6 02", "a6 02", "a6 02", "a6 02", "a6 02"]);
    lines.extend(["c6 03", "a6 03", "a6 03", "a6 03", "a6 03", "a6 03"]);
}

#[test]
fn init_matches_hantek1008py_sequence() {
    let mut scope = Scope::with_transport(FakeDevice::with_burst(constant_burst(2040)));
    let config = Config {
        channels: [true, true, false, false, false, false, false, false],
        scales: [ScaleId::V1; CHANNELS],
        time_div: TimeDivId::from_nanos(1_000_000).unwrap(),
        trigger: Trigger {
            channel: 1,
            slope: Slope::Falling,
            level: 0x0abc,
        },
    };
    let report = scope.init_with(&config).unwrap();

    let mut expected = vec![
        // init 1
        "b0",
        "b0",
        "f3",
        "b9 01 b0 04 00 00",
        "b7 00",
        "bb 08 00",
        "b5",
        "b6",
        "e5",
        "f7",
        "f8",
        "fa",
        "f5",
        "a0 08",
        "aa 01 01 01 01 01 01 01 01",
        "a3 11",
        "c1 00 00",
        "a7 00 00",
        "ac 01 f4 00 09 c5 00 09 c5",
    ];
    // init 2: one forced capture per scale ID 1..=3
    for scale in ["01", "02", "03"] {
        expected.push("f3");
        expected.push(match scale {
            "01" => "a2 01 01 01 01 01 01 01 01",
            "02" => "a2 02 02 02 02 02 02 02 02",
            _ => "a2 03 03 03 03 03 03 03 03",
        });
        expected.extend(["a4 01", "c0", "c2", "a5 5a"]);
        burst_reads(&mut expected);
    }
    // init 3
    expected.extend([
        "f6",
        "e5",
        "f7",
        "f8",
        "fa",
        "a3 12",
        "ac 00 c8 00 02 bd 00 02 bd",
        "e4 01",
        "e6 01",
        "f3",
        "a0 02",
        "aa 01 01 00 00 00 00 00 00",
        "a2 03 03 03 03 03 03 03 03",
        "a3 12",
        "c1 01 01",
        "a7 00 00",
        "ac 00 00 00 00 01 00 05 79",
        "ab 0a bc",
        "e9",
    ]);
    assert_eq!(scope.transport().requests(), expected);

    assert_eq!(report.firmware_version, [0x01, 0x09]);
    assert_eq!(report.cal_temperature, [0xd6, 0x06]);
    assert_eq!(report.blobs.len(), 5);
    assert_eq!(report.blobs[4].1.len(), 56);
    assert_eq!(report.e6.len(), 10);
    for scale in ScaleId::ALL {
        for ch in 0..CHANNELS {
            assert_eq!(report.zero_offsets.get(scale, ch), 2040.0 + ch as f64);
        }
    }
    assert_eq!(scope.config(), &config);

    // B0 twice with 0.7 s between; F5/F6/A2 wait ~0.21 s for the reply.
    let sleeps = &scope.transport().sleeps;
    assert!(sleeps.contains(&Duration::from_millis(700)));
    assert!(sleeps.contains(&Duration::from_micros(213_200)));
}

#[test]
fn burst_capture_deinterleaves_active_channels() {
    // Two active channels: frames (100+i, 200+i), 50 frames = 200 bytes,
    // 100 B per block -> 2 chunks each with a partial final chunk.
    let burst: Vec<u16> = (0..50).flat_map(|i| [100 + i, 200 + i]).collect();
    let mut scope = Scope::with_transport(FakeDevice::with_burst(burst));
    let config = Config {
        channels: [false, false, true, false, false, true, false, false],
        ..Config::default()
    };
    scope.configure(&config).unwrap();
    let n_config = scope.transport().log.len();

    let data = scope.capture_burst().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0], (0..50).map(|i| 100 + i).collect::<Vec<u16>>());
    assert_eq!(data[1], (0..50).map(|i| 200 + i).collect::<Vec<u16>>());

    let requests = scope.transport().requests();
    assert_eq!(
        &requests[n_config..],
        [
            "f3", "e4 01", "e6 01", "a4 01", "c0", "c2", "a5 5a", "c6 02", "a6 02", "a6 02",
            "c6 03", "a6 03", "a6 03", "e4 01", "e6 01",
        ]
    );
}

#[test]
fn ready_polling_pings_between_checks() {
    let mut dev = FakeDevice::with_burst(vec![1, 2]);
    dev.not_ready_polls = 2;
    dev.polls_left = 2;
    let mut scope = Scope::with_transport(dev);
    let config = Config {
        channels: [true, false, false, false, false, false, false, false],
        ..Config::default()
    };
    scope.configure(&config).unwrap();
    let data = scope.capture_burst().unwrap();
    assert_eq!(data, [vec![1, 2]]);
    let requests = scope.transport().requests().join(",");
    assert!(
        requests.contains("a5 5a,f3,a5 5a,f3,a5 5a,c6 02"),
        "{requests}"
    );
}

#[test]
fn wait_trigger_mode_skips_force_and_gives_up() {
    let mut dev = FakeDevice::with_burst(vec![1, 2]);
    dev.not_ready_polls = u32::MAX;
    dev.polls_left = u32::MAX;
    let mut scope = Scope::with_transport(dev);
    let err = scope
        .capture_burst_with(TriggerMode::Wait(Duration::from_millis(100)))
        .unwrap_err();
    assert!(matches!(err, Error::NotReady(5)), "{err}");
    let requests = scope.transport().requests();
    assert!(requests.contains(&"c0".to_string()));
    assert!(!requests.contains(&"c2".to_string()));
}

#[test]
fn roll_mode_drops_ninth_channel() {
    let mut dev = FakeDevice::default();
    // Two channels + extra value per frame; 30 frames = 180 bytes (3 chunks).
    let frames: Vec<u16> = (0..30).flat_map(|i| [10 + i, 20 + i, 1742]).collect();
    dev.roll
        .push_back(frames.iter().flat_map(|s| s.to_le_bytes()).collect());
    let mut scope = Scope::with_transport(dev);
    let config = Config {
        channels: [true, true, false, false, false, false, false, false],
        ..Config::default()
    };
    scope.configure(&config).unwrap();

    assert!(matches!(scope.roll_read(), Err(Error::NotRolling)));
    let n_config = scope.transport().log.len();
    scope.roll_start(RollRateId::new(0x18).unwrap()).unwrap();
    let data = scope.roll_read().unwrap();
    assert_eq!(data[0], (0..30).map(|i| 10 + i).collect::<Vec<u16>>());
    assert_eq!(data[1], (0..30).map(|i| 20 + i).collect::<Vec<u16>>());
    assert_eq!(
        &scope.transport().requests()[n_config..],
        [
            "a3 18", "f3", "a4 02", "c0", "c2", "f3", "c7", "c8", "c8", "c8"
        ]
    );
    scope.roll_stop().unwrap();
}

#[test]
fn roll_length_must_be_whole_rows() {
    let mut dev = FakeDevice::default();
    dev.roll.push_back(vec![0u8; 10]); // 8 channels + 1 -> 18-byte rows
    let mut scope = Scope::with_transport(dev);
    scope.roll_start(RollRateId::default()).unwrap();
    let err = scope.roll_read().unwrap_err();
    assert!(err.to_string().contains("not a multiple"), "{err}");
}

#[test]
fn echo_mismatch_is_reported() {
    let dev = FakeDevice {
        corrupt_echo: Some(0x00),
        ..FakeDevice::default()
    };
    let mut scope = Scope::with_transport(dev);
    let err = scope.ping().unwrap_err();
    assert!(err.to_string().contains("echo byte is 0x00"), "{err}");
}

#[test]
fn close_sends_ping_and_f4() {
    let dev = FakeDevice::default();
    let log = Rc::clone(&dev.shared_log);
    // The leading drain read finds nothing pending and times out harmlessly.
    Scope::with_transport(dev).close().unwrap();
    assert_eq!(*log.borrow(), ["f3", "f4"]);
}

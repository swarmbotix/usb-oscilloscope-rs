# Hantek 1008 cross-platform driver — implementation plan

Goal: a Rust userspace driver for the Hantek 1008/1008C (USB `0783:5725`) that runs on
Linux (Ubuntu 24), Windows, macOS and Android, with one protocol implementation shared by a
desktop CLI and a Flutter app. Everything is built by GitHub Actions from one repository.

Repository layout:

```
usb-oscilloscope-rs/     # this repository = the Cargo workspace
  Cargo.toml
  crates/                # usbscope-proto, usbscope-usb, usbscope-cli (later: usbscope-ffi)
  docs/                  # protocol reference, per-OS setup, hardware test checklist
  windows/, linux/       # WinUSB INF, udev rule
  app/                   # Flutter app (phase 5)
  .github/workflows/     # CI: build + test every target on each push
  PLAN.md                # this file
```

The vendor's original installer, drivers and manuals are kept outside this repository
(they are Hantek's, not ours to redistribute); the protocol facts below were recovered from
them.

---

## 0. What we already know (inputs to the design)

From the static analysis of `HTNTLVDll.dll` / `Scope.exe` and the open-source
[hantek1008py](https://github.com/mfg92/hantek1008py) (Apache-2.0) project:

| Fact | Source |
|---|---|
| USB `VID 0x0783`, `PID 0x5725`; one bulk OUT + one bulk IN endpoint, full-speed, 64-byte packets | driver INF, HTNTLVDll, hantek1008py |
| Every request is `[cmd, params…]`; the reply's first byte echoes `cmd` (a few reads have no echo) | HTNTLVDll disassembly |
| No firmware upload at connect; device runs its own STM32 firmware (`E9` returns version) | HTNTLVDll (`dsoGetARMVersion`) |
| Data is read as 64-byte chunks = 32 little-endian 16-bit samples, two blocks (param 2 then 3), interleaved across active channels | HTNTLVDll (`dsoReadNormalData`), hantek1008py |
| Windows vendor driver `STM32MS.sys` exposes device interface GUID `{4B6C73B2-015B-4855-8BB1-A7A6C03AFA87}` and is driven with plain Read/Write | HTNTLVDll + driver GUID match |

### Command reference (to be implemented in `usbscope-proto`)

| Cmd | Purpose | Request | Reply |
|---|---|---|---|
| `B0` | init / reset (send twice, ~0.7 s apart) | `B0` | `B0` |
| `E9` | firmware (ARM) version | `E9` | 2 bytes, **no echo** (hantek1008py sees `01 09`; do not assert on it) |
| `E4 01`, `E6 01` | init-time steps seen only in hantek1008py (layout unknown) | 2 bytes | `E4` echo / 10 bytes no echo |
| `E5` | calibration temperature | `E5` | 2 bytes, no echo |
| `B5`,`B6`,`F7`,`F8`,`FA` | init-time calibration/config blobs | 1 byte | 64/64/…/56 bytes, no echo |
| `F5`,`F6` | init steps | 1 byte | echo, ~0.2 s latency |
| `F3` | keep-alive ping (every ~10 ms while idle) | `F3` | `F3` |
| `F4` | close | `F4` | `F4` |
| `A0 n` | active channel count | `A0 n` (7-byte generic form also used for DIO/freq/trigger/div in Scope.exe) | echo (10 bytes for generic form) |
| `AA m1…m8` | channel enable map (1/0 per channel) | 9 bytes | `AA` |
| `A2 s1…s8` | vertical scale ID per channel (1 = 0.02 V/unit most sensitive, 2 = 0.125, 3 = 1.0 — order per hantek1008py) | 9 bytes | `A2` |
| `A3 id` | time/div ID (burst, 0–25 = 1/2/5×10ⁿ ns) or sample-rate ID (roll, `0x18`–`0x24` = 440 Hz … 1/16 Hz) | 2 bytes | `A3` |
| `A4 m` | mode: 1 = burst, 2 = roll | 2 bytes | `A4` |
| `A5 5A` | ready check (reply 2 or 3 when ready) | 2 bytes | `A5 x` |
| `A7 00 00` | unknown; must reply `00` | 3 bytes | 2 bytes |
| `AB hi lo` | trigger level, 12-bit big-endian | 3 bytes | `AB` |
| `AC …` | fixed config blob (9 bytes) | 9 bytes | `AC` |
| `C1 ch slope` | trigger source (0–7) and slope (0 rising, 1 falling) | 3 bytes | `C1` |
| `C0` / `C2` | arm / force trigger (hantek1008py always sends `C2` right after `C0`; real triggering untested) | 1 byte | echo |
| `C6 b` | burst: sample byte-count for block `b` ∈ {2,3} | 2 bytes | `hi lo`, **no echo** |
| `A6 b` | burst: next 64-byte chunk of block `b` | 2 bytes | 64 bytes |
| `C7` | roll: bytes ready | 1 byte | `hi lo` |
| `C8` | roll: next 64-byte chunk | 1 byte | 64 bytes |
| `B7 x` | generator on/off | 2 bytes | `B7` |
| `B9 01 len32le` | generator pulse length (µs) | 6 bytes | `B9` |
| `BB 08 x` | generator output enable | 3 bytes | `BB` |
| `BF`, `B8 01 data…` | generator waveform upload (62-byte pages) | 64 bytes | echo |
| `E7`/`E8`/`EA` | set language / get language / save settings to device | 3/1/1 bytes | echo |
| `D0`–`D3`, `F0` | factory test commands — **not implemented** | | |

Unknowns to resolve with a USB capture (phase 2): exact layout of the generic 7-byte `A0`
packet used for digital I/O, the `AC` blob, calibration write (`A1`-based pages), and the
effect of `A5`/`A7` ordering.

---

## 1. Phase 1 — Rust workspace and protocol crate (no hardware needed)

Deliverable: `cargo test` passes on all desktops; no USB code yet.

1. `Cargo.toml` workspace, edition 2024, MSRV = stable, `rust-toolchain.toml`.
2. Crate `usbscope-proto` (`#![no_std]`-compatible core + `alloc`):
   - `cmd.rs`: `enum Command` with a `encode(&self, &mut [u8; 64]) -> usize` and the expected
     reply length / echo flag for each command from the table above.
   - `tables.rs`: time/div IDs, roll-rate IDs, vertical scale IDs, per-channel-count
     sample-rate factors (`[4.56, 3.03, 2.27, 1.82, 1.51, 1.30, 1.14, 1.00]`).
   - `frame.rs`: reply validation (`echo` check), 16-bit LE sample decoding,
     de-interleave by active channel count, roll-mode 9th-channel drop.
   - `calib.rs`: zero-offset averaging per scale, linear-interpolated correction factors,
     raw→volts conversion (`(raw - zero) * 0.01 * scale`).
   - `session.rs`: a sans-I/O state machine (`Idle → Configured → Armed → Reading`) that
     yields the next request and consumes replies, so the full init and capture sequences are
     unit-testable against recorded byte streams.
   - Tests: golden byte vectors for every command; a recorded init+burst transcript
     (hand-assembled from hantek1008py's documented replies now, replaced with a real capture
     in phase 2).
3. Error type with `thiserror` (desktop) / plain enum (`no_std`); `defmt`-free, `log` facade.

## 2. Phase 2 — USB transport and desktop CLI (first hardware contact)

Deliverable: `usbscope identify` prints the firmware version on Ubuntu and Windows;
`usbscope capture --ch 1,2 --tdiv 1ms --out a.csv` produces plausible data.

1. Crate `usbscope-usb` on [`nusb`](https://crates.io/crates/nusb) (pure Rust; Linux usbfs,
   Windows WinUSB, macOS IOKit, Android via fd):
   - `Device::open()` — enumerate `0783:5725`, claim interface 0, find first bulk OUT/IN.
   - `Device::from_fd(raw_fd)` — Android/Linux path (`nusb::Device::from_fd`).
   - `Transport` trait: `write(&[u8])`, `read(len, timeout)`; the proto session drives it.
   - Timeouts: 1 s default, 5 s for `F5`/`F6`; retry policy only on `Pipe` errors after `A3`.
   - Keep-alive thread/task sending `F3` every 10 ms while idle (as the vendor app does).
2. Crate `usbscope-cli` (`clap`): `list`, `identify`, `capture` (burst), `roll`, `gen`.
   CSV output compatible with hantek1008py's `csvexport.py` columns.
3. Windows driver binding: ship `windows/hantek1008-winusb.inf` (WinUSB,
   `USB\VID_0783&PID_5725`) plus instructions for Zadig as the alternative. Document that this
   replaces Hantek's `STM32MS.sys` binding and how to revert using `proprietary/Driver`.
4. Linux: `linux/99-hantek1008.rules` (`MODE="0666"`).
5. **Validation loop** (needs the physical scope):
   - Capture Scope.exe traffic once with USBPcap + Wireshark on Windows while doing: connect,
     set CH1 1 V/div, 1 ms/div, single capture, roll mode, generator on. Save as
     `captures/*.pcapng` (git-tracked, small).
   - Convert to golden transcripts for the proto tests; diff our request bytes against the
     vendor's; fix tables/ordering until identical.
   - Run the CLI on Ubuntu 24 and Windows against the scope; compare CSV against Scope.exe's
     own CSV export for the same signal (probe-compensation square wave).

## 3. Phase 3 — FFI surface

Deliverable: one shared library per platform with a stable C ABI.

1. Crate `usbscope-ffi` (`crate-type = ["cdylib", "staticlib"]`):
   - Handle-based C API: `ht_open`, `ht_open_fd`, `ht_close`, `ht_version`, `ht_configure`,
     `ht_capture_burst`, `ht_roll_start/next/stop`, `ht_generator_*`, `ht_last_error`.
   - Generated header via `cbindgen` (`include/usbscope.h`).
   - Opaque handles, no panics across the boundary (`catch_unwind`), all buffers caller-owned.
2. Also expose the same API through `flutter_rust_bridge` annotations (async-friendly Dart
   bindings) — the FFI crate hosts both so there is one place to maintain.

## 4. Phase 4 — CI: build everything at once (GitHub Actions)

`.github/workflows/build.yml`, matrix on every push and PR:

| Job | Runner | Targets | Steps |
|---|---|---|---|
| `test` | ubuntu-latest | host | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace` |
| `linux` | ubuntu-latest | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (cross) | build CLI + cdylib, upload artifacts |
| `windows` | windows-latest | `x86_64-pc-windows-msvc` | build CLI + dll, bundle INF |
| `macos` | macos-latest | `aarch64-apple-darwin`, `x86_64-apple-darwin` | build CLI + dylib |
| `android` | ubuntu-latest | `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` | `cargo-ndk` with NDK r27, upload `.so`s |
| `release` | on tag `v*` | all of the above | attach artifacts to the GitHub release |

Tooling: `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `cargo-ndk`, `cross` for the
aarch64 Linux build. Hardware tests cannot run in CI; they stay manual (phase 2 checklist).

## 5. Phase 5 — Flutter app (Android first, then desktop)

Deliverable: `app/` Flutter project that identifies the scope and shows live CH1–CH8 traces.

1. `flutter create app`, add `flutter_rust_bridge` codegen against `usbscope-ffi`.
2. Android USB plumbing (the only non-Rust, non-Dart code):
   - `AndroidManifest.xml`: `uses-feature android.hardware.usb.host`, intent filter
     `USB_DEVICE_ATTACHED` with `device_filter.xml` (`0783:5725`).
   - ~30 lines Kotlin: `UsbManager.requestPermission`, `openDevice()`, return
     `connection.fileDescriptor` to Dart via a MethodChannel; Dart passes it to `ht_open_fd`.
3. Desktop targets reuse `ht_open` directly (no permission step).
4. UI: channel enable/scale panel, time/div, trigger, run/stop, CSV export, generator page.
   Rendering via `CustomPainter`; decimate to screen width in Rust before crossing FFI.
5. Add `app` jobs to the CI matrix: `flutter build apk`, `flutter build linux|windows|macos`.

## 6. Phase 6 — hardening and docs

- Firmware-version tolerance (do not assert `01 09`; log and continue).
- Reconnect on unplug; clean `F4` on exit; `reset()` on open if the device is wedged.
- Calibration persistence (`~/.config/hantek1008/calib.toml`; app settings on Android).
- README with per-OS setup (udev rule, WinUSB INF/Zadig, OTG notes), protocol doc generated
  from the proto crate's command table.
- Optional: `pyo3` wheel so hantek1008py users can switch backends.

---

## Order of work and checkpoints

1. Phase 1 → `cargo test` green (no hardware). **Checkpoint: review protocol crate API.**
2. Phase 2 → USB capture of Scope.exe, CLI `identify` works on Windows and Ubuntu.
   **Checkpoint: first real data from the scope; compare CSV with vendor export.**
3. Phase 4 CI early (right after phase 2) so every later change is built for all targets.
4. Phase 3 FFI → Phase 5 Android app → Phase 6.

## Risks

- Vendor driver vs WinUSB conflict on Windows (one-time binding switch; documented revert).
- Unknown fields (`A0` generic packet, `AC`, calibration writes) — unblock with the capture.
- Timing-sensitive init (`B0` ×2 with 0.7 s, `F5`/`F6` latency, keep-alive) — copy vendor
  timings exactly first, relax later with measurements.
- Android OTG power: some phones cannot power the scope; recommend a powered hub.
- No hardware in CI: protocol correctness is guarded by transcript tests, not live runs.

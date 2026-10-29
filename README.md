# usb-oscilloscope-rs

A cross-platform userspace driver and command-line tool, written in Rust, for the
**Hantek 1008 / 1008C** 8-channel USB oscilloscope (USB ID `0783:5725`).

- Runs on **Linux, Windows, macOS** and (as a library) **Android**, from one codebase.
- No vendor DLLs and no kernel driver of its own: it talks to the scope directly over USB
  using the pure-Rust [`nusb`](https://crates.io/crates/nusb) library (Linux usbfs,
  Windows WinUSB, macOS IOKit, Android via a file descriptor).
- One protocol implementation, shared by the CLI today and an FFI layer / Flutter app later.

> **Status: alpha.** The USB protocol was reconstructed from a disassembly of Hantek's
> Windows DLL and from the hardware-tested Python project
> [hantek1008py](https://github.com/mfg92/hantek1008py). It has **not yet been validated
> against a real scope.** Expect rough edges until the
> [hardware test checklist](docs/hardware-test-checklist.md) has been completed. Reports
> from anyone with a Hantek 1008 are very welcome.

## Quick start

1. **Get the binary.** Download `usbscope` for your platform from the GitHub Releases page
   (or the artifacts of a CI run), or build it:

   ```sh
   cd usb-oscilloscope-rs
   cargo build --release -p usbscope-cli
   # binary: target/release/usbscope (usbscope.exe on Windows)
   ```

   You need a current stable Rust toolchain (1.97 at the time of writing; the workspace
   uses edition 2024). Get it from <https://rustup.rs>.

2. **Give it access to the scope** (once per machine):

   | OS | What to do | Guide |
   |---|---|---|
   | Linux | Install a udev rule so normal users can open the device. | [docs/setup-linux.md](docs/setup-linux.md) |
   | Windows | Switch the scope from Hantek's driver to WinUSB (Zadig, or the INF in `windows/`). Reversible. | [docs/setup-windows.md](docs/setup-windows.md) |
   | macOS | Nothing. Possibly a powered USB hub. | [docs/setup-macos.md](docs/setup-macos.md) |
   | Android | USB host (OTG) and an app that passes a file descriptor to the library. No app yet. | [docs/setup-android.md](docs/setup-android.md) |

3. **Run it:**

   ```sh
   usbscope list
   usbscope identify
   usbscope capture --channels 1,2 --tdiv 1ms --scale 1 --out out.csv
   ```

## The `usbscope` command

Global options: `-v` / `-vv` for more logging (`-vv` prints every USB request and reply in
hex), `--timeout-ms <ms>` for the USB read timeout. Run `usbscope <command> --help` for the
full, authoritative list of options.

| Command | What it does |
|---|---|
| `usbscope list` | Lists connected Hantek 1008 scopes. |
| `usbscope identify [--index N]` | Opens scope number `N` (default: the first), runs the full init sequence, and prints the firmware version, the calibration temperature and the factory calibration blobs in hex. |
| `usbscope capture --channels 1,2 --tdiv 1ms --scale 1 --out out.csv` | Takes a single-shot (burst) capture of the given channels and writes it as CSV. |
| `... [--count N]` | Takes `N` captures in a row. |
| `... [--trigger-ch 1 --slope rising --level 2048]` | Trigger source channel, slope (`rising`/`falling`) and level in raw ADC units (0..4096, 2048 = mid-scale). |
| `... [--raw]` | Writes raw ADC values instead of volts. |
| `usbscope raw <hex> [--reply-len N] [--no-echo]` | Sends one raw command (for example `e9`) and prints the reply. For protocol debugging; see [docs/protocol.md](docs/protocol.md). Do not send commands you do not understand: some write to the device's flash. |

### Example session

The output below is illustrative; exact formatting may differ between versions.

```text
$ usbscope list
[0] 0783:5725  Hantek 1008  (bus 1, address 7)

$ usbscope identify
Firmware version : 01 09
Calib temperature: d6 06
B5: 00080008000800080008000800080008d407c907ef07cd07...
B6: 04040404040404040404040404040404d200d500d800d400...
F7: ...
F8: ...
FA: ...

$ usbscope capture --channels 1,2 --tdiv 1ms --scale 1 --out out.csv
Captured 1 frame, 2 channels -> out.csv

$ usbscope raw f3
> f3
< f3
```

## Repository layout

```
usb-oscilloscope-rs/
  Cargo.toml                # workspace
  crates/
    usbscope-proto/         # sans-I/O protocol: command encoding, reply parsing, value tables,
                            #   calibration math, session state machine. No USB code.
    usbscope-usb/           # USB transport on nusb: open by VID/PID, or from an Android fd
                            #   (Scope::from_fd); keep-alive; timeouts.
    usbscope-cli/           # the `usbscope` binary
  docs/
    protocol.md             # the USB protocol, standalone
    setup-linux.md, setup-windows.md, setup-macos.md, setup-android.md
    hardware-test-checklist.md
  linux/99-hantek1008.rules # udev rule
  windows/                  # WinUSB INF + install/revert instructions
```

CI (`.github/workflows/build.yml` at the repository root) checks formatting, runs clippy
and the tests, and builds `usbscope` for Linux (x86_64, aarch64), Windows (x86_64) and
macOS (Apple Silicon, Intel), plus the transport crate for three Android ABIs, on every
push. Tagged versions (`v*`) are published as GitHub Releases.

## Relationship to hantek1008py

[hantek1008py](https://github.com/mfg92/hantek1008py) by mfg92 is a Python driver for the
same scope, tested on real hardware. This project owes it the meaning of most protocol
fields, the value tables (time/div, roll-mode sample rates, vertical scales, sample-rate
factors) and the init timing. Those were cross-checked against a disassembly of Hantek's
`HTNTLVDll.dll`; see [docs/protocol.md](docs/protocol.md) for which facts come from where.

Differences in approach:

- Rust instead of Python, single static binary, no `libusb` dependency.
- Windows, macOS and Android are first-class targets.
- The protocol logic is sans-I/O (no USB calls inside), so it is unit-tested against
  recorded byte transcripts in CI, without hardware.
- Per-unit calibration replies are recorded and printed instead of asserted against one
  unit's values.
- The CSV output aims to stay compatible with hantek1008py's `csvexport.py` columns where
  practical.

## Roadmap

See [PLAN.md](PLAN.md) for the full plan, including the C ABI / `flutter_rust_bridge`
FFI crate, the Flutter app (Android first, then desktop), roll mode and generator support
in the CLI, and calibration persistence.

## License

To be decided. hantek1008py is licensed under Apache-2.0; its protocol tables and timing
information are used here with credit (see above and [docs/protocol.md](docs/protocol.md)).

Hantek is a trademark of its owner. This project is not affiliated with or endorsed by
Hantek.

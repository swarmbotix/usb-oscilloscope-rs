# Hardware test checklist

The protocol implementation was written from a disassembly of Hantek's DLL and from
[hantek1008py](https://github.com/mfg92/hantek1008py), without access to a scope. This
checklist is the ordered procedure for validating it against a real Hantek 1008/1008C. Do
the steps in order: each one assumes the previous one passed.

Record the results (pass/fail, `usbscope` version or git commit, firmware version, OS) in
the issue or pull request you are working on.

## What you need

- A Hantek 1008 or 1008C, its USB cable, and at least one probe.
- A **Windows PC with Hantek's software** (`Scope.exe`) installed, for the reference
  capture. Steps 1 and 2 need it.
- [USBPcap](https://desowin.org/usbpcap/) and [Wireshark](https://www.wireshark.org/) on
  that PC (Wireshark's installer can install USBPcap for you).
- A known test signal. Best: the scope's calibration / probe-compensation square-wave
  output, if your unit has one. Otherwise: one of the scope's own generator outputs, a
  function generator, or a known DC source (for example a fresh 1.5 V battery).
- `usbscope` built or downloaded for the OS under test (see the `setup-*.md` pages).

## Step 1: Reference USB capture of the vendor software (do this first)

Everything else is compared against this capture, so do it before switching drivers.

1. Keep the scope on **Hantek's driver**. Close `Scope.exe`.
2. Start Wireshark and pick the **USBPcap** interface of the root hub the scope is plugged
   into. (If unsure, capture on all USBPcap interfaces.)
3. Start the capture, then start `Scope.exe` and wait until it shows traces.
4. In `Scope.exe`, do each of these, with a few seconds' pause in between so the actions
   are easy to find in the capture:
   1. CH1 on, all other channels off.
   2. CH1 to 1 V/div, then to each other volts/div setting once (this reveals the `A2`
      scale IDs).
   3. Time base to 1 ms/div.
   4. A single (one-shot) capture of the test signal on CH1.
   5. Roll mode for about 10 seconds, then stop.
   6. Turn the digital generator on, then off.
   7. **Export the single capture to CSV** with the vendor software's export function.
      Keep this file; you need it in step 6.
5. Close `Scope.exe`, stop the capture.
6. Apply the display filter `usb.idVendor == 0x0783` to find the device address. (This
   matches the device descriptor, which is only in the capture if the scope was plugged
   in after the capture started, or if USBPcap's "inject already connected devices"
   option was on. If nothing matches, look for bulk packets whose first byte is `B0`.)
   Then filter on that address (for example `usb.device_address == 7`) and save as
   **pcapng** under `captures/` with a descriptive name, such as
   `scopeexe-1ms-ch1-single-roll-gen.pcapng`.

What to check in the capture, against [protocol.md](protocol.md):

- [ ] The init order (`B0`, pause, `B0`, `F3`, generator setup, `B5`/`B6`/`E5`/`F7`/`F8`/`FA`, `F5`, ...).
- [ ] Whether the `E9` reply starts with `E9` (echo) or not, and its length.
- [ ] Whether the `C6` reply has an echo byte.
- [ ] The `A2` bytes for each volts/div setting (scale ID order: is ID 1 the most sensitive?).
- [ ] The `A3` byte for 1 ms/div (expected `12`).
- [ ] The keep-alive interval of `F3` while idle (expected ~10 ms).
- [ ] The `AC` and generic 7-byte `A0` packets (layouts are unknown; just record them).

Open questions that the capture answers are listed in [protocol.md section 8](protocol.md#8-open-questions).

## Step 2: Switch the driver (Windows only)

Bind the scope to WinUSB (Zadig, or the INF in `windows/`), as described in
[setup-windows.md](setup-windows.md). On Linux, install the udev rule
([setup-linux.md](setup-linux.md)). macOS needs nothing.

## Step 3: `usbscope list`

```sh
usbscope list
```

- [ ] Exactly one device with ID `0783:5725` is listed.
- [ ] Unplug the scope: the list is empty. Replug: it is back.

If nothing is listed: on Windows the scope is still bound to the Hantek driver; on Linux
check permissions (`ls -l /dev/bus/usb/...`).

## Step 4: `usbscope identify`

```sh
usbscope -vv identify
```

`-vv` logs every request and reply in hex, which is what you compare with the capture.

- [ ] It completes without error. Expect it to take a couple of seconds (`B0` twice with a
      0.7 s pause, and ~0.2 s waits after `F5`, `F6` and `A2`).
- [ ] **Firmware version** is printed. hantek1008py's unit reported `01 09`. A different
      value is **not** a failure: record it.
- [ ] **Calibration temperature** (`E5`) is printed (hantek1008py: `D6 06`).
- [ ] The init blobs (`B5`, `B6`, `F7`, `F8`, `FA`) are printed in hex and are **identical
      to the replies in your step 1 capture** for the same unit. (They are per-unit
      factory data, so they will not match hantek1008py's values.)
- [ ] The request bytes in the `-vv` log match `Scope.exe`'s requests in the capture, in
      the same order. Differences are fine where hantek1008py's order is used on purpose;
      note them.
- [ ] Running `identify` twice in a row works (tests clean close with `F4` and reopen).

## Step 5: `usbscope raw e9`

Low-level check of the framing, without the full init sequence:

```sh
usbscope raw e9
usbscope raw e9 --reply-len 3 --no-echo
usbscope raw e9 --reply-len 2 --no-echo
```

- [ ] Note which form returns sensible data. If the 3-byte read starts with `E9`, the
      firmware echoes `E9`; if the 2-byte read returns `01 09` (or your version), it does
      not. Report the result so `protocol.md` and the proto crate can be corrected.
- [ ] If `raw` gets no reply at all, run `usbscope identify` first (the device may need the
      `B0` init) and try again.
- [ ] `usbscope raw f3` returns `F3` (simplest echo test).

## Step 6: `usbscope capture` against the vendor CSV

Connect the test signal to CH1 (and, if you can, the same signal or ground to CH2).
Use the same settings as the single capture you exported in step 1:

```sh
usbscope capture --channels 1,2 --tdiv 1ms --scale 1 --out ours.csv
usbscope capture --channels 1,2 --tdiv 1ms --scale 1 --out ours-raw.csv --raw
```

Check `usbscope capture --help` for the accepted `--scale` values and pick the one that
corresponds to the volts/div you used in `Scope.exe`.

Compare `ours.csv` with the vendor CSV export (open both in a spreadsheet or plot them):

- [ ] **Sample count** per channel is the same, or explainably different (the vendor
      software may trim or pad).
- [ ] **Time step** between samples matches (about 1 ms / 25 = 40 µs at 1 ms/div; note
      that the effective sample rate rises when fewer channels are active).
- [ ] **Zero level:** with the probe grounded, the mean of `ours.csv` is within a few mV of
      0 V. In `ours-raw.csv`, raw values sit near 2048.
- [ ] **Amplitude:** peak-to-peak voltage of the square wave agrees with the vendor CSV
      within about 5 %. Larger, constant ratios (for example ×8 or ×50) point to a wrong
      scale ID mapping.
- [ ] **Frequency** of the square wave agrees.
- [ ] **Channel order:** the CH1 signal shows up in the CH1 column, not CH2 (checks the
      de-interleaving).
- [ ] Repeat with a single channel (`--channels 1`) and with all eight
      (`--channels 1,2,3,4,5,6,7,8`).
- [ ] `--count 10` produces 10 captures without errors or growing delays.
- [ ] Trigger: `--trigger-ch 1 --slope rising --level 2048` and then `--slope falling`;
      check whether the waveform start moves to the edge. (The known-good sequence forces
      the trigger, so a non-moving start is a finding to report, not necessarily a bug.)

## Step 7: Cross-platform repeat

- [ ] Repeat steps 3 to 6 on Ubuntu 24.04 (and macOS if available) with the same scope.
      The init blobs and CSV results should match the Windows run.

## Known timing pitfalls

| Symptom | Cause | What to do |
|---|---|---|
| **Pipe error** (endpoint stall) on `C0` right after entering roll mode | `A3` was not sent with a roll-rate ID (`18`..`24`) before `A4 02`. hantek1008py documents this. | Always send `A3 <rate>` before `A4 02`. If it happens, clear the halt, drain, re-init from `B0`. |
| Firmware version is not `01 09` | Different firmware revision. | Not an error. Record it. Only treat it as a problem if later steps fail too. |
| `B5`/`F7`/`F8`/`FA` replies differ from hantek1008py | Per-unit factory calibration. | Expected. Compare with your own `Scope.exe` capture instead. |
| Timeout on `A2`, `F5` or `F6` | These take ~0.21 s before replying. | Use a longer `--timeout-ms` (for example `--timeout-ms 5000`) to confirm. |
| `A5 5A` keeps answering 0 or 1 | Acquisition not finished (slow time base) or not armed. | It is polled up to ~20 times with `F3` and 20 ms pauses; at very slow time bases more attempts are needed. |
| First reply after a crash is garbage, or the echo byte is wrong | A reply from an interrupted command is still queued. | Unplug/replug, or drain the IN endpoint before init (the driver does this on open). |
| Device stops answering after idling | No `F3` keep-alive for a long time. | The driver sends `F3` every ~10 ms while idle; check `-vv` logs for gaps. |
| Random disconnects, especially on laptops or phones | USB power. | Use a powered hub. |

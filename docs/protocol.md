# Hantek 1008 / 1008C USB protocol

This is the USB protocol spoken by the Hantek 1008 and 1008C 8-channel USB oscilloscopes,
as implemented by the `usbscope-proto` crate. It is written to stand on its own: you should
be able to talk to the scope with nothing but this page and a USB library.

**Status:** reverse-engineered, not official. Nothing here comes from Hantek documentation.
Treat every byte as "believed correct" until it has been checked against a real USB capture
(see [hardware-test-checklist.md](hardware-test-checklist.md)).

## Where this information comes from

Two independent sources were combined:

1. **Vendor DLL disassembly.** Hantek's Windows software (`Scope.exe`) talks to the scope
   through `HTNTLVDll.dll`. Static analysis of that DLL gave the list of command bytes, the
   size of each request and reply, and which replies echo the command byte.
2. **[hantek1008py](https://github.com/mfg92/hantek1008py)** by mfg92 (Apache-2.0), a
   Python driver tested on real hardware. It documents what the fields mean, the value
   tables (time/div, sample rates, vertical scales) and the init timing. The field
   semantics and value tables below come from that project.

The **Source** column in the command tables uses these labels:

| Label | Meaning |
|---|---|
| **both** | Opcode and packet sizes appear in the DLL, and hantek1008py uses the command on real hardware with the same meaning. Highest confidence. |
| **DLL** | Seen only in the vendor DLL. Opcode and packet size are known; the meaning is inferred from the DLL function name and has not been hardware-tested. |
| **py** | Seen only in hantek1008py (hardware-tested), not identified in the DLL analysis. |
| **unknown layout** | The command exists, but the meaning of some or all parameter bytes is not known. Send the bytes exactly as recorded. |

## 1. The device on the bus

| Property | Value |
|---|---|
| Vendor ID / Product ID | `0x0783` / `0x5725` |
| USB speed | Full speed (12 Mbit/s) |
| Configuration / interface | Configuration 1, interface 0, alternate setting 0 |
| Endpoints | One bulk OUT and one bulk IN endpoint. Use the first OUT and the first IN endpoint of interface 0, and read their addresses from the descriptors instead of hard-coding them. |
| Max packet size | 64 bytes |
| Class | Vendor-specific. No standard OS driver claims it. |
| Firmware | The scope runs its own STM32 firmware from flash. **No firmware upload is needed** when it is plugged in (unlike many cheap USB scopes). |

On Windows the vendor driver is `STM32MS.sys` (shipped renamed as `HANTEK1008*.sys`). It
registers device interface GUID `{4B6C73B2-015B-4855-8BB1-A7A6C03AFA87}` and is driven with
plain `ReadFile`/`WriteFile` calls, which map one-to-one onto the bulk transfers described
here. This project replaces that driver with Microsoft's WinUSB (see
[setup-windows.md](setup-windows.md)).

## 2. Framing rules

1. **One request = one bulk OUT transfer** of 1 to 64 bytes: `[cmd, param1, param2, ...]`.
   There is no length prefix, checksum or sequence number.
2. **One reply = one bulk IN read** whose length is fixed per command (see the tables).
   The host must know the expected length in advance and ask for exactly that many bytes.
3. **Echo rule.** Most replies start with the command byte (`cmd`), followed by the
   payload. A reply that should echo but does not start with `cmd` means the host and
   device are out of step. Some "read" commands reply **without** the echo byte: `E5`,
   `E6`, `E9`, `B5`, `B6`, `F7`, `F8`, `FA`, `C6`, `A6`, `C7`, `C8`. These are marked
   *no echo* in the tables.
4. **Strictly one outstanding request.** Send a request, read its reply, then send the
   next. Never pipeline.
5. **Every request gets a reply.** Even "set" commands reply with the echo byte. If a
   request is written but its reply is never read (for example because the program was
   interrupted), the device keeps that reply queued and the next read returns stale data.
   On connect and before close, drain the IN endpoint with a short read (hantek1008py uses
   64 bytes with a 100 ms timeout) and ignore whatever comes back.
6. **Byte order.** Control fields are big-endian (`AB` trigger level, `C6`/`C7` byte
   counts). Sample data and the generator fields (`B9`, `BF`) are little-endian. Each case
   is spelled out below.
7. **Pacing.** hantek1008py waits about 2 ms before each write. A few commands are slow
   and need a long read timeout (or a delay before reading): `A2`, `F5` and `F6` take
   about **0.21 s**, `B5` about 20 ms. A 1 s default read timeout, with 5 s for `F5`/`F6`,
   is safe.
8. **Keep-alive.** While the scope is idle, the host sends `F3` roughly every **10 ms**.
   Both the vendor software and hantek1008py do this. What happens if you stop is not
   documented; assume the device may time out and lose its state.

## 3. Command reference

Notation: `XX` is one byte in hex, `n` is a number. Request and reply lengths are in bytes
and **include** the command/echo byte.

### 3.1 Session and identification

| Cmd | Name | Request | Reply | Notes | Source |
|---|---|---|---|---|---|
| `B0` | Init / reset | `B0` (1) | `B0` (1) | Sent **twice**, about 0.7 s apart, at the start of every session. | both |
| `F3` | Keep-alive ping | `F3` (1) | `F3` (1) | Every ~10 ms while idle, and before most capture steps. | both |
| `F4` | Close | `F4` (1) | `F4` (1) | Last command before releasing the device. hantek1008py also issues a USB port reset afterwards. | both |
| `E9` | Firmware (ARM) version | `E9` (1) | 2 bytes, *no echo* | hantek1008py reads exactly 2 bytes and gets `01 09`. Other units may report a different version: log it, do not reject it. See section 8 on the echo question. | both |
| `E5` | Calibration temperature (raw) | `E5` (1) | 2 bytes, *no echo* | hantek1008py sees `D6 06`. How to convert the value to degrees is not known. | both |
| `F5` | Init step 1 | `F5` (1) | `F5` (1) | About 0.21 s before the reply is ready. | both |
| `F6` | Init step 2 | `F6` (1) | `F6` (1) | About 0.21 s before the reply is ready. | both |

### 3.2 Factory calibration / config blobs (read at init)

These return per-unit factory data. hantek1008py reads them (and asserts on the exact bytes
of its author's unit) but does not decode them. **Do not compare them with fixed values**:
they will differ between units. Store and print them; `usbscope identify` does this.

| Cmd | Request | Reply | Notes | Source |
|---|---|---|---|---|
| `B5` | `B5` (1) | 64 bytes, *no echo* | ~20 ms latency. Looks like 16-bit LE values around `0x0800` (2048), plausibly per-channel zero levels. | both |
| `B6` | `B6` (1) | 64 bytes, *no echo* | | both |
| `F7` | `F7` (1) | 64 bytes, *no echo* | | both |
| `F8` | `F8` (1) | 64 bytes, *no echo* | | both |
| `FA` | `FA` (1) | **56** bytes, *no echo* | | both |

### 3.3 Acquisition setup

| Cmd | Name | Request | Reply | Notes | Source |
|---|---|---|---|---|---|
| `A0` | Active channel count | `A0 n` (2), `n` = 1..8 | `A0` (1) | Always followed by `AA`. | both |
| `A0` (generic) | Generic 7-byte form | `A0` + 6 bytes (7) | 10 bytes | Used by `Scope.exe` for digital I/O, the frequency counter, trigger and div settings. | DLL, **unknown layout** |
| `AA` | Channel enable map | `AA m1 m2 ... m8` (9); `mi` = `01` on, `00` off | `AA` (1) | `m1` is channel 1. | both |
| `A2` | Vertical scale per channel | `A2 s1 ... s8` (9); `si` = scale ID 1..3 (section 4.3) | `A2` (1) | ~0.21 s latency. | both |
| `A3` | Time base | `A3 id` (2) | `A3` (1) | Burst mode: time/div ID `00`..`19` (section 4.1). Roll mode: sample-rate ID `18`..`24` (section 4.2). The two ranges overlap; the meaning depends on the mode set with `A4`. | both |
| `A4` | Acquisition mode | `A4 m` (2); `01` = burst, `02` = roll | `A4` (1) | | both |
| `A7` | Unknown | `A7 00 00` (3) | `A7 00` (2) | The reply payload has always been `00`. Sent during init after `C1`. | both, **unknown layout** |
| `AB` | Trigger level | `AB hi lo` (3) | `AB` (1) | Big-endian, 0..4096 in raw ADC units; 2048 is mid-scale. | both |
| `AC` | Config blob | `AC` + 8 bytes (9) | `AC` (1) | Meaning unknown. hantek1008py sends three different fixed blobs during init (section 5.1); copy them exactly. | both, **unknown layout** |
| `C1` | Trigger source and slope | `C1 ch slope` (3); `ch` = 0..7, `slope` = `00` rising, `01` falling | `C1` (1) | `ch` is zero-based (0 = CH1). | both |
| `E4` | Unknown | `E4 01` (2) | `E4` (1) | Sent together with `E6` around captures. hantek1008py says they are "not necessarily required". | py, **unknown layout** |
| `E6` | Unknown | `E6 01` (2) | 10 bytes, *no echo* | Looks like five 16-bit LE values near `0x06E5`. Possibly temperature or supply monitoring. | py, **unknown layout** |

### 3.4 Capture

| Cmd | Name | Request | Reply | Notes | Source |
|---|---|---|---|---|---|
| `C0` | Arm (start acquisition) | `C0` (1) | `C0` (1) | | both |
| `C2` | Force trigger | `C2` (1) | `C2` (1) | hantek1008py always sends `C2` right after `C0`, so it never waits for a real trigger. Waiting for a trigger (sending `C0`, skipping `C2`, polling `A5 5A`) is untested. | both |
| `A5` | Ready check | `A5 5A` (2) | `A5 x` (2) | `x` is 0..3. **2 or 3 = data ready**; 0 or 1 = not yet. Poll up to ~20 times, sending `F3` and waiting ~20 ms between attempts. | both |
| `C6` | Burst: block byte count | `C6 b` (2); `b` = `02` or `03` | 2 bytes, *no echo*, big-endian | Number of sample **bytes** in block `b`. | both |
| `A6` | Burst: read chunk | `A6 b` (2) | 64 bytes, *no echo* | Repeat `ceil(count / 64)` times for the same block, then truncate to `count` bytes. | both |
| `C7` | Roll: bytes ready | `C7` (1) | 2 bytes, *no echo*, big-endian | 0 means nothing ready yet. Always a multiple of `(active_channels + 1) * 2`. | both |
| `C8` | Roll: read chunk | `C8` (1) | 64 bytes, *no echo* | Repeat `ceil(ready / 64)` times; the last chunk is zero-padded, so truncate to `ready` bytes. | both |

### 3.5 Digital pattern generator

The 1008 has an 8-bit digital output port (the "programmable generator"). These commands
are less tested than the rest: hantek1008py marks its generator code as "not tested" apart
from the values it sends during init.

| Cmd | Name | Request | Reply | Notes | Source |
|---|---|---|---|---|---|
| `B7` | Generator stop / prepare | `B7 x` (2) | `B7` (1) | hantek1008py always sends `B7 00` before changing generator settings. | both |
| `B9` | Generator step length | `B9 01 L0 L1 L2 L3` (6); `L` = 32-bit little-endian | `B9` (1) | Time per waveform step. Init sends `B9 01 B0 04 00 00` (1200). The unit (µs per step?) is not confirmed. Large values switch the first byte to `02` with a different, unknown encoding. | both |
| `BB` | Generator output enable | `BB 08 x` (3); `x` = `01` on, `00` off | `BB` (1) | | both |
| `BF` | Waveform length | `BF n0 n1` (3); 16-bit little-endian length in bytes | `BF` (1) | At most 1440 steps. | both |
| `B8` | Waveform data page | `B8 01` + 62 data bytes (64) | `B8` (1) | One byte per step; bit *i* drives output *i*. Pad unused bytes with `00`. Waveforms longer than 62 bytes need several pages; the page numbering is not confirmed. | both |

### 3.6 Settings storage and factory commands

| Cmd | Name | Request | Reply | Notes | Source |
|---|---|---|---|---|---|
| `E7` | Set language | 3 bytes | echo | UI language stored in the device for `Scope.exe`. | DLL |
| `E8` | Get language | 1 byte | echo + payload | | DLL |
| `EA` | Save settings to device | 1 byte | echo | Writes to device flash. Avoid. | DLL |
| `A1` | Calibration write (paged) | unknown | unknown | Writes factory calibration. **Not implemented. Do not send:** it can overwrite the unit's calibration. | DLL, **unknown layout** |
| `D0`..`D3`, `F0` | Factory test | | | **Not implemented. Do not send.** | DLL |

## 4. Value tables

### 4.1 Time/div IDs (burst mode, `A3 id`)

`id = 3 * decade + k`, with `k` = 0 for ×1, 1 for ×2, 2 for ×5, starting at 1 ns/div. A
division holds roughly 25 samples. The fastest settings are accepted by the firmware but
are far beyond what the ADC can actually sample; treat anything below about 1 µs/div as
unverified.

| ID (hex) | time/div | ID (hex) | time/div | ID (hex) | time/div |
|---|---|---|---|---|---|
| `00` | 1 ns | `09` | 1 µs | `12` | 1 ms |
| `01` | 2 ns | `0A` | 2 µs | `13` | 2 ms |
| `02` | 5 ns | `0B` | 5 µs | `14` | 5 ms |
| `03` | 10 ns | `0C` | 10 µs | `15` | 10 ms |
| `04` | 20 ns | `0D` | 20 µs | `16` | 20 ms |
| `05` | 50 ns | `0E` | 50 µs | `17` | 50 ms |
| `06` | 100 ns | `0F` | 100 µs | `18` | 100 ms |
| `07` | 200 ns | `10` | 200 µs | `19` | 200 ms |
| `08` | 500 ns | `11` | 500 µs (vendor default) | | |

For time bases slower than 200 ms/div, use roll mode.

### 4.2 Roll-mode sample-rate IDs (`A3 id`, then `A4 02`)

| ID (hex) | Nominal rate | ID (hex) | Nominal rate |
|---|---|---|---|
| `18` | 440 Hz | `1F` | 2 Hz |
| `19` | 220 Hz | `20` | 1 Hz |
| `1A` | 88 Hz | `21` | 0.5 Hz |
| `1B` | 44 Hz | `22` | 0.25 Hz |
| `1C` | 22 Hz | `23` | 0.125 Hz |
| `1D` | 11 Hz | `24` | 1/16 Hz (0.0625 Hz) |
| `1E` | 5 Hz | | |

The nominal rate is per channel **with all 8 channels active**. With fewer active channels
the device samples faster. Multiply the nominal rate by:

| Active channels | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| Rate factor | 4.56 | 3.03 | 2.27 | 1.82 | 1.51 | 1.30 | 1.14 | 1.00 |

hantek1008py measured these factors in roll mode; they are approximate. Example: 2 channels
at ID `18` give about 440 × 3.03 ≈ 1333 samples/s per channel.

### 4.3 Vertical scale IDs (`A2`)

| ID | Scale factor | Volts per raw ADC count (`0.01 × factor`) | Approx. range (±2048 counts) |
|---|---|---|---|
| 1 | 0.02 | 0.2 mV | ±0.4 V |
| 2 | 0.125 | 1.25 mV | ±2.5 V |
| 3 | 1.0 | 10 mV | ±20 V |

This mapping is taken from hantek1008py's source (`__VSCALE_FACTORS = [0.02, 0.125, 1.0]`,
ID = list index + 1): **ID 1 is the most sensitive range and ID 3 the least sensitive.**
The range column is arithmetic, not measured; the analog front end may clip earlier.

## 5. Sequences

The sequences below are what hantek1008py does on real hardware, which in turn copies the
vendor software. Follow them in order; other orders are untested. `->` shows the expected
reply.

### 5.1 Connect and initialise

```
(drain any stale IN data, ignore errors)
B0                          -> B0
(wait 0.7 s)
B0                          -> B0
F3                          -> F3
B9 01 B0 04 00 00           -> B9          generator step length 1200
B7 00                       -> B7          generator off ...
BB 08 00                    -> BB          ... and output disabled
B5                          -> 64 bytes    (no echo, ~20 ms)
B6                          -> 64 bytes    (no echo)
E5                          -> 2 bytes     calibration temperature (no echo)
F7                          -> 64 bytes    (no echo)
F8                          -> 64 bytes    (no echo)
FA                          -> 56 bytes    (no echo)
F5                          -> F5          (~0.21 s)
A0 08                       -> A0          all 8 channels ...
AA 01 01 01 01 01 01 01 01  -> AA          ... enabled
A3 11                       -> A3          500 us/div (vendor default)
C1 00 00                    -> C1          trigger on CH1, rising
A7 00 00                    -> A7 00
AC 01 F4 00 09 C5 00 09 C5  -> AC
```

**Zero-offset calibration.** Repeat for scale ID 1, 2 and 3, with no signal connected:

```
F3                          -> F3
A2 s s s s s s s s          -> A2          all channels at scale ID s (~0.21 s)
A4 01                       -> A4          burst mode
C0                          -> C0
(wait ~12 ms)
C2                          -> C2
A5 5A (poll)                -> A5 02 or A5 03
burst read of blocks 2 and 3 (section 5.2)
```

The average of each channel's samples is that channel's zero offset for scale `s`. Keep all
24 values (8 channels × 3 scales).

**Second init phase:**

```
F6                          -> F6          (~0.21 s)
E5, F7, F8, FA              (read again; same replies as before)
A3 <tdiv id>                -> A3
AC 00 C8 00 02 BD 00 02 BD  -> AC
E4 01                       -> E4
E6 01                       -> 10 bytes    (no echo)
F3                          -> F3
A0 <n>                      -> A0          the user's channel selection
AA <map>                    -> AA
A2 <scales>                 -> A2
A3 <tdiv id>                -> A3
C1 <ch> <slope>             -> C1
A7 00 00                    -> A7 00
AC 00 00 00 00 01 00 05 79  -> AC
AB <level hi> <level lo>    -> AB
E9                          -> 2 bytes     firmware version (no echo)
```

After this, start the `F3` keep-alive (every ~10 ms) until the next capture.

### 5.2 Burst (single-shot) capture

```
(pause keep-alive)
F3                          -> F3
E4 01 / E6 01               (optional, see table)
A4 01                       -> A4          (~15 ms before reading the reply)
C0                          -> C0          arm
C2                          -> C2          force trigger
A5 5A (poll until 2 or 3)   -> A5 x
C6 02                       -> hi lo       byte count N2 (no echo)
A6 02, ceil(N2/64) times    -> 64 bytes each; keep the first N2 bytes
C6 03                       -> hi lo       byte count N3 (no echo)
A6 03, ceil(N3/64) times    -> 64 bytes each; keep the first N3 bytes
E4 01 / E6 01               (optional)
(resume keep-alive)
```

The sample data is block 2 followed by block 3.

### 5.3 Roll (continuous) capture

```
A3 <roll rate id 18..24>    -> A3          MUST come before A4 02, see below
(wait ~10 ms)
F3                          -> F3
A4 02                       -> A4          roll mode
C0                          -> C0
C2                          -> C2
loop:
    F3                      -> F3
    C7                      -> hi lo       bytes ready R (no echo); repeat until R > 0
    C8, ceil(R/64) times    -> 64 bytes each; keep the first R bytes
    decode (section 6)
```

**Pitfall:** hantek1008py reports a USB **pipe error** (the endpoint stalls) on `C0` when
`A3` was not sent with a roll-rate ID before switching to roll mode. Always send `A3` first.

### 5.4 Close

```
(stop keep-alive)
(drain any stale IN data with a short timeout)
F3                          -> F3
F4                          -> F4
(optional: USB port reset, as hantek1008py does)
```

## 6. Sample data format

1. **Sample word.** Two bytes, **little-endian**, unsigned: `value = b[0] + 256 * b[1]`.
   The ADC is 12-bit; with no input the value sits near 2048 (mid-scale).
2. **Interleaving.** Samples rotate through the **active** channels in ascending channel
   order. With channels {1, 3, 4} active, the stream is `ch1, ch3, ch4, ch1, ch3, ch4, ...`.
   Active channel number *k* (0-based) of *n* gets `s[k], s[k + n], s[k + 2n], ...`.
3. **Roll mode has an extra 9th slot.** In roll mode each row holds `n + 1` samples: the
   active channels, then one extra value (around 1742 on hantek1008py's unit, meaning
   unknown). Use a stride of `n + 1` and drop the last slot of each row.
4. **Raw to volts.**

   ```
   volts = (raw - zero_offset[channel][scale]) * 0.01 * scale_factor
   ```

   `zero_offset` comes from the zero-offset calibration in section 5.1, and `scale_factor`
   is 0.02, 0.125 or 1.0 (section 4.3). hantek1008py can additionally multiply by a
   per-channel correction factor, obtained by measuring known reference voltages and
   interpolating linearly between them. Without that, expect a few percent of gain error.
5. **Trigger level** uses the same raw units. The channel's zero offset (about 2048) is
   0 V, so a level in volts is `(level - zero_offset) * 0.01 * scale_factor`.

## 7. Endpoint errors and recovery

- A **STALL / pipe error** usually means a command was sent out of order (see the roll-mode
  pitfall). Clear the halt, drain the IN endpoint and restart from `B0`.
- A **timeout** on a slow command (`A2`, `F5`, `F6`) usually means the read timeout is too
  short, not that the device failed.
- A reply that should echo but does not start with the command byte means an earlier reply
  was never read. Drain and resynchronise.

## 8. Open questions

These need a USB capture of `Scope.exe` (USBPcap + Wireshark) to settle:

- **`E9` echo.** The DLL analysis suggested a 3-byte reply `E9 lo hi`; hantek1008py reads 2
  bytes without echo and gets `01 09`. This page uses the hardware-tested form (2 bytes, no
  echo). To check: `usbscope raw e9 --reply-len 3 --no-echo`. If the first byte is `E9`,
  the echo exists.
- **`C6` echo.** Same question; the hardware-tested form is 2 bytes, no echo.
- **Vertical scale ID order.** An earlier draft of the plan listed ID 1 as the 1 V range;
  hantek1008py's source says ID 1 = factor 0.02 (most sensitive). Confirm by capturing the
  `A2` bytes `Scope.exe` sends for each volts/div setting.
- Layout of the generic 7-byte `A0` packet, the `AC` blob, `E4`/`E6`, and the `A1`
  calibration-write pages.
- Whether a real (non-forced) trigger works when `C2` is omitted.
- The meaning of `A5` reply values 0..3, and whether the order of `A5`/`A7` matters.
- The 9th roll-mode slot and the contents of the `B5`/`B6`/`F7`/`F8`/`FA` blobs.

## Credits

Field meanings, value tables and timing come from
[hantek1008py](https://github.com/mfg92/hantek1008py) by mfg92, licensed under Apache-2.0.
The opcode list and packet sizes were cross-checked against static analysis of Hantek's
`HTNTLVDll.dll`. Hantek is a trademark of its owner; this project is not affiliated with
Hantek.

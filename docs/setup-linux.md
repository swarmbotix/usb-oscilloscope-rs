# Setup on Linux (tested target: Ubuntu 24.04)

Summary: install one udev rule so your user may open the scope, then run `usbscope`.
No kernel driver is needed; `usbscope` talks to the scope directly through the kernel's
`usbfs` interface.

## 1. Get `usbscope`

Either:

- **Download** `usbscope-x86_64-unknown-linux-gnu.tar.gz` (or the `aarch64` one for a
  Raspberry Pi 4/5 or other 64-bit ARM board) from the project's GitHub Releases page and
  unpack it:

  ```sh
  tar xzf usbscope-x86_64-unknown-linux-gnu.tar.gz
  ./usbscope --help
  ```

- **Build from source.** Install Rust from <https://rustup.rs>, plus a C linker
  (`sudo apt install build-essential`), then:

  ```sh
  cd usb-oscilloscope-rs
  cargo build --release -p usbscope-cli
  ./target/release/usbscope --help
  ```

  No `libusb` or `libudev` development packages are needed: the USB layer (`nusb`) is pure
  Rust.

## 2. Allow your user to access the scope (udev rule)

By default, Linux only lets `root` open raw USB devices. Without the rule you will see
"Permission denied" and would have to run `sudo usbscope ...`, which is not recommended.

The rule file is `linux/99-hantek1008.rules` in this repository. It contains:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="0783", ATTR{idProduct}=="5725", MODE="0666", TAG+="uaccess"
```

It matches the scope by its USB IDs and makes its device node readable and writable by all
users.

Install it:

```sh
sudo cp linux/99-hantek1008.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then **unplug and replug** the scope. (Reloading only affects devices that are added after
the reload; `udevadm trigger` usually covers the already-plugged device too, but replugging
is the reliable way.)

### Check the rule took effect

```sh
lsusb -d 0783:5725
# Bus 001 Device 007: ID 0783:5725 ...
ls -l /dev/bus/usb/001/007      # use the Bus and Device numbers from lsusb
# crw-rw-rw- 1 root root 189, 6 ... /dev/bus/usb/001/007
```

The permissions should be `crw-rw-rw-`. If they are still `crw-rw-r--`, check the file name
ends in `.rules`, look for typos, and run `udevadm test $(udevadm info -q path -n /dev/bus/usb/001/007)`
to see which rules were applied.

### Stricter alternative

`MODE="0666"` gives every local account access. On a shared machine you may prefer access
for the logged-in desktop user only: rename the file to `70-hantek1008.rules` and delete
`MODE="0666",` from it. The `uaccess` tag then grants access to whoever is logged in at the
physical seat (this only works for rule files numbered below 73, which is why the default
99- file relies on `MODE`).

## 3. Check that it works

```sh
usbscope list
usbscope identify
```

`list` should show one device with ID `0783:5725`. `identify` initialises the scope (a
couple of seconds) and prints the firmware version and calibration data.

## Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| `Permission denied` / `Access denied` | The udev rule is not active. Re-check section 2 and replug. |
| `Device or resource busy` | Another process has the scope open (a second `usbscope`, or hantek1008py). Close it. |
| `lsusb` does not show `0783:5725` | Cable, port or power. Try another port; avoid unpowered hubs. `sudo dmesg -w` while plugging in shows whether the kernel sees the device at all. |
| Timeouts or "pipe error" during capture | See the timing pitfalls in [hardware-test-checklist.md](hardware-test-checklist.md). Run with `-vv` to log every USB request and reply. |
| Works with `sudo` only | Same as "Permission denied": the rule is not applied. |

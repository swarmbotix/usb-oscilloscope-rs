# Setup on macOS

Summary: **no driver and no configuration are needed.** macOS lets ordinary programs open
vendor-specific USB devices like this scope through IOKit, which is what `usbscope` (via the
`nusb` library) uses. Hantek ships no macOS software, so there is nothing to uninstall.

## 1. Get `usbscope`

Either:

- **Download** the build for your Mac from the project's GitHub Releases page:
  `usbscope-aarch64-apple-darwin.tar.gz` for Apple Silicon (M1 and later) or
  `usbscope-x86_64-apple-darwin.tar.gz` for Intel Macs. Unpack it:

  ```sh
  tar xzf usbscope-aarch64-apple-darwin.tar.gz
  ```

  The binary is not signed or notarised. Files downloaded with a browser are quarantined by
  Gatekeeper, and macOS will refuse to run it ("cannot be opened because the developer
  cannot be verified"). Remove the quarantine flag once:

  ```sh
  xattr -d com.apple.quarantine ./usbscope
  ./usbscope --help
  ```

- **Build from source.** Install the Xcode command line tools (`xcode-select --install`) and
  Rust from <https://rustup.rs>, then:

  ```sh
  cd usb-oscilloscope-rs
  cargo build --release -p usbscope-cli
  ./target/release/usbscope --help
  ```

## 2. Check that it works

```sh
usbscope list
usbscope identify
```

You can also confirm macOS sees the scope at all with:

```sh
system_profiler SPUSBDataType | grep -B2 -A8 -i 0x0783
```

## Power and hubs

The scope is powered from USB. Some Macs, USB-C adapters and unpowered hubs cannot supply
enough current, which shows up as the scope disappearing during init or captures that time
out. If that happens, connect it through a **powered USB hub**.

## Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| "cannot be opened because the developer cannot be verified" | Remove the quarantine flag (section 1), or right-click the file in Finder → Open once. |
| `usbscope list` is empty | Check `system_profiler SPUSBDataType`. If the scope is not listed there either, try another cable, adapter or a powered hub. |
| "exclusive access" / "resource busy" | Another program has the device open. Close it and retry. |
| Timeouts during capture | See [hardware-test-checklist.md](hardware-test-checklist.md); run with `-vv`. |

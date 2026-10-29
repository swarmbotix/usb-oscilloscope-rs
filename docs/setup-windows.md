# Setup on Windows 10 / 11

Summary: switch the scope from Hantek's driver to Microsoft's **WinUSB** driver (once),
then run `usbscope.exe`.

## 1. Get `usbscope.exe`

Either:

- **Download** `usbscope-x86_64-pc-windows-msvc.zip` from the project's GitHub Releases page
  (or from the artifacts of a CI run) and unzip it. The zip also contains the WinUSB INF.
- **Build from source.** Install Rust from <https://rustup.rs> (the MSVC toolchain; the
  installer offers to install the Visual Studio Build Tools if they are missing), then:

  ```powershell
  cd usb-oscilloscope-rs
  cargo build --release -p usbscope-cli
  .\target\release\usbscope.exe --help
  ```

## 2. Why the driver has to change

On Windows, every USB device is owned by exactly one driver. Hantek's installer binds the
scope to its own kernel driver, `STM32MS.sys` (installed under a name like
`HANTEK1008*.sys`). That driver exposes its own private interface
(`{4B6C73B2-015B-4855-8BB1-A7A6C03AFA87}`), which only Hantek's DLL knows how to use.

`usbscope` uses the [`nusb`](https://crates.io/crates/nusb) library, which on Windows talks
to devices through **WinUSB**, Microsoft's generic driver for vendor-specific USB devices.
WinUSB ships with Windows, so nothing has to be installed except a small description file
(an INF) that tells Windows "use WinUSB for USB ID 0783:5725".

Consequence: **only one program family can use the scope at a time.** With WinUSB, Hantek's
`Scope.exe` cannot see the scope; with the Hantek driver, `usbscope` cannot. Switching back
and forth takes a minute and is fully reversible.

## 3. Bind the scope to WinUSB

Pick one:

- **Zadig (recommended, about 1 minute).** Run [Zadig](https://zadig.akeo.ie/), choose
  *Options → List All Devices*, select the device with USB ID `0783 5725`, pick **WinUSB**
  as the target driver and click *Replace Driver*.
- **The INF in this repository.** `windows/hantek1008-winusb.inf` is a ready-made WinUSB
  package. Because it is not signed, you must create and sign a catalog file for it with a
  self-made certificate before Windows accepts it. This needs the Windows Driver Kit and
  SDK.

Step-by-step instructions for both are in [`windows/README.md`](../windows/README.md).

## 4. Check that it works

Plug in the scope and run:

```powershell
usbscope list
usbscope identify
```

`list` should show one device with ID `0783:5725`. `identify` initialises the scope (it
takes a couple of seconds because the init sequence contains deliberate pauses) and prints
the firmware version.

## 5. Switching back to Hantek's software

In Device Manager, right-click the scope → *Update driver → Browse my computer → Let me
pick from a list* → choose the Hantek entry. If no Hantek entry is listed, uninstall the
device (ticking *Attempt to remove the driver for this device*) and reinstall Hantek's
software from Hantek's website. Details: [`windows/README.md`](../windows/README.md#switching-back-to-the-hantek-driver).

## Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| `usbscope list` shows nothing, but the scope is plugged in | The scope is still bound to the Hantek driver. Check Device Manager: under *Universal Serial Bus devices* it should say "WinUSB" or our device name. Redo step 3. |
| "Access denied" / "resource busy" when opening | Another program has the device open (for example a second `usbscope`, or Zadig still running). Close it and retry. |
| Zadig does not list the scope | Enable *Options → List All Devices*. Try another USB port or cable. |
| `identify` times out | Unplug the scope for a few seconds and retry. Run with `-vv` to see each request and reply, and check [hardware-test-checklist.md](hardware-test-checklist.md). |
| The scope disconnects at random | Use a rear-panel port or a powered hub; front-panel ports and long cables sometimes cannot supply enough current. |

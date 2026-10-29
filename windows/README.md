# Windows driver binding for the Hantek 1008

`usbscope` talks to the scope through Microsoft's generic **WinUSB** driver. Out of the box,
Windows binds the scope to Hantek's own driver (`STM32MS.sys`, installed as
`HANTEK1008*.sys`), which `usbscope` cannot use. You need to switch the scope over to
WinUSB once. You can switch back at any time.

> **Only one driver at a time.** While the scope is bound to WinUSB, Hantek's `Scope.exe`
> will not find it. While it is bound to the Hantek driver, `usbscope` will not find it.

This folder contains:

| File | What it is |
|---|---|
| `hantek1008-winusb.inf` | A WinUSB driver package for `USB\VID_0783&PID_5725`. It copies no files; it tells Windows to use the inbox `winusb.sys`. |

There are two ways to do the switch. **Option A (Zadig) is recommended** for most people.

---

## Option A: Zadig (recommended)

[Zadig](https://zadig.akeo.ie/) is a small, widely used tool that generates, signs and
installs a WinUSB package for one device in a few clicks.

1. Plug in the scope.
2. Download and run Zadig (it asks for administrator rights).
3. Menu **Options → List All Devices**.
4. In the drop-down, choose the Hantek scope. Check that the **USB ID** field shows
   `0783 5725`. If you are not sure which entry it is, unplug the scope and see which one
   disappears.
5. Left of the green arrow you see the current driver (for example `HANTEK1008...` or
   `STM32MS`). On the right, select **WinUSB**.
6. Click **Replace Driver** (or **Install Driver**) and wait for "The driver was installed
   successfully".
7. Unplug and replug the scope, then run `usbscope list`.

## Option B: install this INF yourself

Use this if you cannot or do not want to use Zadig, for example on a managed machine.

### Why an extra step is needed

64-bit Windows refuses to install a driver package that is not signed. This INF only
references Microsoft's own, already signed `winusb.sys`, but the **package** (the INF plus
its catalog file) still needs a signature. We cannot ship a Microsoft-signed package, so you
create a catalog and sign it with a self-made certificate that only your PC trusts. (Zadig
does exactly this behind the scenes.)

You need:

- the **Windows Driver Kit (WDK)** for `Inf2Cat.exe`, and
- the **Windows SDK** for `signtool.exe`.

Both are free downloads from Microsoft. The commands below assume they are on `PATH`, or
that you run them from a "Developer PowerShell for VS" window.

### Steps (PowerShell, run as Administrator, inside this `windows` folder)

```powershell
# 1. Create the catalog file hantek1008-winusb.cat next to the INF.
#    Use /os:10_X64 on x64 Windows. For ARM64, run `Inf2Cat /?` to see the OS names.
Inf2Cat /driver:. /os:10_X64

# 2. Create a self-signed code-signing certificate in the machine store.
$cert = New-SelfSignedCertificate -Type CodeSigningCert `
    -Subject "CN=usb-oscilloscope-rs local driver" `
    -CertStoreLocation Cert:\LocalMachine\My

# 3. Trust it on this PC only (Root + TrustedPublisher).
Export-Certificate -Cert $cert -FilePath .\usbscope-local.cer
Import-Certificate -FilePath .\usbscope-local.cer -CertStoreLocation Cert:\LocalMachine\Root
Import-Certificate -FilePath .\usbscope-local.cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher

# 4. Sign the catalog.
signtool sign /fd SHA256 /sm /s My /sha1 $cert.Thumbprint .\hantek1008-winusb.cat

# 5. Add the package to the driver store and bind it to the plugged-in scope.
pnputil /add-driver .\hantek1008-winusb.inf /install
```

Then unplug and replug the scope. In Device Manager it should appear under
**Universal Serial Bus devices** as *Hantek 1008 USB Oscilloscope (WinUSB)*.

If Windows still picks the Hantek driver, open Device Manager, right-click the scope,
choose **Update driver → Browse my computer → Let me pick from a list**, and select
*Hantek 1008 USB Oscilloscope (WinUSB)*.

### About test-signing mode

You should **not** need to enable test-signing mode (`bcdedit /set testsigning on`),
because no unsigned kernel binary is involved. Only consider it as a last resort if the
steps above are rejected by a strict policy. It requires Secure Boot to be turned off, it
puts a "Test Mode" watermark on the desktop, and you should turn it off again afterwards
with `bcdedit /set testsigning off`.

---

## Switching back to the Hantek driver

Do this when you want to use Hantek's `Scope.exe` again (for example to record a
reference capture).

### Quick way: pick the other driver

If the Hantek driver is still installed on the PC (it usually is, if you ever installed
Hantek's software):

1. Open **Device Manager** and find the scope under **Universal Serial Bus devices**.
2. Right-click → **Update driver → Browse my computer for drivers → Let me pick from a list
   of available drivers on my computer**.
3. Select the Hantek entry and click **Next**.

### Clean way: remove the WinUSB binding

1. In **Device Manager**, right-click the scope → **Uninstall device**.
2. Tick **Attempt to remove the driver for this device** and click **Uninstall**.
3. Remove the package from the driver store as well (PowerShell as Administrator):

   ```powershell
   pnputil /enum-drivers
   # Find the entry whose "Original Name" is hantek1008-winusb.inf (Option B) or whose
   # provider is "libwdi" for the Hantek device (Zadig), note its "Published Name"
   # (oemNN.inf), then:
   pnputil /delete-driver oemNN.inf /uninstall
   ```

4. Unplug and replug the scope. Windows now falls back to the Hantek driver if it is in the
   driver store.
5. If the scope shows up as an unknown device instead, reinstall Hantek's driver. Download
   the Hantek 1008 software from Hantek's website and run its setup, or point **Update
   driver → Browse my computer** at the folder that contains Hantek's `.inf`. (Maintainers
   keep a local copy of the vendor driver in `proprietary/Driver/` at the repository root;
   that folder is git-ignored and not distributed, so everyone else should use Hantek's
   download.)

### Optional: remove the self-signed certificate (Option B only)

```powershell
Get-ChildItem Cert:\LocalMachine\Root, Cert:\LocalMachine\TrustedPublisher, Cert:\LocalMachine\My |
    Where-Object Subject -eq "CN=usb-oscilloscope-rs local driver" |
    Remove-Item
```

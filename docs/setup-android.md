# Using the scope on Android

**Status:** the Rust transport (`usbscope-usb`) compiles for Android today and can open the
scope from a file descriptor. There is no ready-made Android app yet: the Flutter app is a
later phase of the project (see [PLAN.md](../PLAN.md), phase 5). This page explains what
an Android app has to do, so you can wire it up yourself or follow along.

The `usbscope` command-line tool is **not** usable on a normal (non-rooted) Android device:
apps cannot open `/dev/bus/usb/...` directly. Access goes through Android's USB host API, as
described below.

## 1. Hardware requirements

- **USB host (OTG) support.** Most phones and tablets from the last decade have it. Some
  vendors (several Samsung, OnePlus and Oppo models, for example) have an *OTG* switch in
  Settings that turns itself off after a few minutes of inactivity; enable it.
- **An OTG adapter or cable**: USB-C (or micro-USB) male to USB-A female, then the scope's
  normal cable.
- **Power.** The scope is powered from the USB port. Many phones cannot supply enough
  current, which shows up as the scope not enumerating, or disconnecting during init. Use a
  **powered USB hub** (or an OTG Y-cable with external power) between phone and scope.

## 2. How an app gets access

Android does not let apps open USB devices by path. Instead:

1. The app declares that it uses USB host mode and which devices it is interested in.
2. Android asks the user for permission (or the user ticks "use by default" once).
3. The app opens the device with `UsbManager.openDevice()` and gets a
   `UsbDeviceConnection`.
4. `UsbDeviceConnection.getFileDescriptor()` returns a Linux file descriptor for the
   already-opened `usbfs` device node. The app passes this number to native code.
5. Rust wraps it with `Scope::from_owned_fd(fd)` (built on `nusb`'s Android/Linux `from_fd`
   support) and from then on speaks the protocol exactly as on desktop Linux.

### `res/xml/device_filter.xml`

Matches the Hantek 1008. Android wants decimal IDs: `0x0783` = 1923, `0x5725` = 22309.

```xml
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <!-- Hantek 1008 / 1008C USB oscilloscope, VID 0x0783, PID 0x5725 -->
    <usb-device vendor-id="1923" product-id="22309" />
</resources>
```

### `AndroidManifest.xml` entries

```xml
<manifest ...>
    <!-- The app needs USB host mode. Set required="false" if it should also install on
         devices without it. -->
    <uses-feature android:name="android.hardware.usb.host" android:required="true" />

    <application ...>
        <activity android:name=".MainActivity" android:exported="true" ...>
            <!-- Offer to open this app when the scope is plugged in. -->
            <intent-filter>
                <action android:name="android.hardware.usb.action.USB_DEVICE_ATTACHED" />
            </intent-filter>
            <meta-data
                android:name="android.hardware.usb.action.USB_DEVICE_ATTACHED"
                android:resource="@xml/device_filter" />
        </activity>
    </application>
</manifest>
```

If the user launches the app through the "USB device attached" prompt and ticks "Always
open", Android grants permission for that device automatically. Otherwise the app must
ask, as below.

### Kotlin: request permission and get the file descriptor

```kotlin
private const val ACTION_USB_PERMISSION = "com.example.usbscope.USB_PERMISSION"

fun openScope(context: Context, onFd: (Int) -> Unit) {
    val usb = context.getSystemService(Context.USB_SERVICE) as UsbManager
    val device = usb.deviceList.values.firstOrNull {
        it.vendorId == 0x0783 && it.productId == 0x5725
    } ?: return  // scope not connected

    fun open() {
        val connection = usb.openDevice(device) ?: return
        // Keep `connection` alive while Rust uses the device; close it when done.
        // Hand Rust its own duplicate of the fd, so each side closes only its own copy.
        val fdForRust = ParcelFileDescriptor.fromFd(connection.fileDescriptor).detachFd()
        onFd(fdForRust)  // e.g. a JNI / flutter_rust_bridge call into Scope::from_owned_fd
    }

    if (usb.hasPermission(device)) { open(); return }

    val receiver = object : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) {
            ctx.unregisterReceiver(this)
            if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) open()
        }
    }
    ContextCompat.registerReceiver(
        context, receiver, IntentFilter(ACTION_USB_PERMISSION),
        ContextCompat.RECEIVER_NOT_EXPORTED,
    )
    // Android 12+ requires FLAG_MUTABLE here (the system adds extras to the intent), and
    // Android 14+ requires the intent to be explicit, hence setPackage().
    val intent = Intent(ACTION_USB_PERMISSION).setPackage(context.packageName)
    val pending = PendingIntent.getBroadcast(context, 0, intent, PendingIntent.FLAG_MUTABLE)
    usb.requestPermission(device, pending)
}
```

### Rust side

```rust
// fd: the integer received from Kotlin (already a duplicate owned by Rust).
use std::os::fd::{FromRawFd, OwnedFd};
let owned = unsafe { OwnedFd::from_raw_fd(fd) };
let scope = Scope::from_owned_fd(owned)?;
```

`usbscope-usb` offers two constructors. `Scope::from_owned_fd(OwnedFd)` takes ownership and
closes the descriptor when the `Scope` is dropped — pair it with the duplicate the Kotlin code
makes above. `unsafe fn Scope::from_fd(RawFd)` instead copies the descriptor and leaves the
original for the caller to close; use it only if you want Kotlin to keep ownership (then do
not `detachFd()`, and keep `connection` open for the lifetime of the `Scope`).

Until the FFI crate (`usbscope-ffi`, PLAN.md phase 3) exists, calling Rust from Kotlin or
Dart means writing your own small JNI or `extern "C"` wrapper around this call.

## 3. Building the Rust code for Android

CI already does this on every push (job `android` in `.github/workflows/build.yml`). To do
it locally you need the Android NDK (r27) and `cargo-ndk`:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=/path/to/android-ndk-r27d

cd usb-oscilloscope-rs
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 build --release -p usbscope-usb
```

This currently produces Rust libraries (`.rlib`) only, which proves the code compiles for
Android. Loadable `.so` files for an app (`jniLibs/<abi>/libusbscope_ffi.so`) will be
produced once the FFI crate exists, with:

```sh
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o target/jniLibs build --release -p usbscope-ffi
```

## Troubleshooting

| Symptom | Likely cause and fix |
|---|---|
| Device list is empty | OTG disabled in Settings, adapter not OTG-capable, or not enough power: try a powered hub. |
| Permission dialog never appears | The `PendingIntent` is not mutable/explicit (Android 12+/14+), or the receiver was registered after `requestPermission`. |
| `openDevice()` returns `null` | Permission was not granted, or another app currently holds the device. |
| Scope disconnects during init | Power. Use a powered hub. |

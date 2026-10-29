//! Desktop CLI implementation (argument definitions and subcommands).

use crate::{csv, parse};

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use usbscope_proto::tables::SAMPLES_PER_DIV;
use usbscope_proto::{CHANNELS, Command, Config, RawCommand, ScaleId, Slope, Trigger};
use usbscope_usb::{InitReport, Scope, TriggerMode, hex};

/// Hantek 1008/1008C USB oscilloscope tool.
#[derive(Parser)]
#[command(name = "usbscope", version, about)]
struct Cli {
    /// More logging (-v debug, -vv trace with every USB transaction in hex).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Per-transfer USB timeout in milliseconds (init steps use at least 5 s).
    #[arg(long, default_value_t = 1000, global = true)]
    timeout_ms: u64,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List attached Hantek 1008 devices.
    List,
    /// Initialise the scope and print firmware version, calibration data and
    /// zero offsets.
    Identify {
        /// Device index from `usbscope list`.
        #[arg(long, default_value_t = 0)]
        index: usize,
    },
    /// Capture bursts and write them to CSV.
    Capture(CaptureArgs),
    /// Send one arbitrary command and print the reply (protocol debugging).
    Raw(RawArgs),
}

#[derive(clap::Args)]
struct CaptureArgs {
    /// Device index from `usbscope list`.
    #[arg(long, default_value_t = 0)]
    index: usize,
    /// Channels to capture, 1-based, comma-separated.
    #[arg(long, value_delimiter = ',', default_value = "1")]
    channels: Vec<u8>,
    /// Time per division, e.g. 1ms, 500us, 200ms (rounded to a 1/2/5 step).
    #[arg(long, default_value = "500us")]
    tdiv: String,
    /// Vertical scale factor: 1, 0.125 or 0.02; one value for all channels or
    /// one per channel.
    #[arg(long, value_delimiter = ',', default_value = "1")]
    scale: Vec<f64>,
    /// Output CSV file.
    #[arg(short, long)]
    out: PathBuf,
    /// Number of bursts to capture.
    #[arg(long, default_value_t = 1)]
    count: usize,
    /// Trigger source channel, 1-based (default: first captured channel).
    #[arg(long)]
    trigger_ch: Option<u8>,
    /// Trigger slope.
    #[arg(long, value_enum, default_value_t = SlopeArg::Rising)]
    slope: SlopeArg,
    /// Trigger level, raw 12-bit ADC counts (2048 = 0 V nominal).
    #[arg(long, default_value_t = 2048)]
    level: u16,
    /// Do not force-trigger after arming; wait for the real trigger instead.
    #[arg(long)]
    no_force: bool,
    /// With --no-force: how long to wait for a trigger, in milliseconds.
    #[arg(long, default_value_t = 10_000)]
    trigger_wait_ms: u64,
    /// Write raw ADC counts instead of volts.
    #[arg(long)]
    raw: bool,
    /// Convert with the zero offsets measured during init instead of the
    /// nominal 2048.
    #[arg(long)]
    measured_zero: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum SlopeArg {
    Rising,
    Falling,
}

#[derive(clap::Args)]
struct RawArgs {
    /// Request bytes in hex: `a5 5a`, `a55a` or `0xa5,0x5a`.
    #[arg(required = true, num_args = 1..)]
    bytes: Vec<String>,
    /// Maximum reply bytes to read (a short packet ends the read early).
    #[arg(long, default_value_t = 64)]
    reply_len: usize,
    /// The reply has no echo byte (only affects the report).
    #[arg(long)]
    no_echo: bool,
    /// Run the full init sequence before sending.
    #[arg(long)]
    init: bool,
    /// Device index from `usbscope list`.
    #[arg(long, default_value_t = 0)]
    index: usize,
}

/// Parse arguments and run the selected subcommand.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
    let timeout = Duration::from_millis(cli.timeout_ms);

    match cli.command {
        Cmd::List => list(),
        Cmd::Identify { index } => with_scope(index, timeout, identify),
        Cmd::Capture(args) => {
            let index = args.index;
            with_scope(index, timeout, |scope| capture(scope, &args))
        }
        Cmd::Raw(args) => with_scope(args.index, timeout, |scope| raw(scope, &args)),
    }
}

/// Open the device, run `f`, and always attempt a clean close afterwards.
fn with_scope(
    index: usize,
    timeout: Duration,
    f: impl FnOnce(&mut Scope) -> Result<()>,
) -> Result<()> {
    let mut scope = open(index)?;
    scope.set_timeout(timeout);
    let result = f(&mut scope);
    let closed = scope.close();
    match (result, closed) {
        (Err(e), Err(close_err)) => {
            log::warn!("close after error also failed: {close_err}");
            Err(e)
        }
        (Err(e), Ok(())) => Err(e),
        (Ok(()), closed) => closed.context("closing the device failed"),
    }
}

fn open(index: usize) -> Result<Scope> {
    Scope::open_nth(index).map_err(|e| {
        let hint = match &e {
            usbscope_usb::Error::NotFound => no_device_hint(),
            usbscope_usb::Error::Usb(_) => access_hint(),
            _ => "",
        };
        anyhow::Error::new(e).context(format!("cannot open Hantek 1008 #{index}{hint}"))
    })
}

fn no_device_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "\nhint: is it plugged in? Check `lsusb | grep 0783:5725`."
    } else {
        "\nhint: is it plugged in and powered?"
    }
}

fn access_hint() -> &'static str {
    if cfg!(target_os = "windows") {
        "\nhint: the device must use the WinUSB driver (install the INF or use Zadig), \
         and no other program (e.g. Hantek Scope.exe) may have it open."
    } else if cfg!(target_os = "linux") {
        "\nhint: install the udev rule granting access to 0783:5725, then replug the device."
    } else {
        ""
    }
}

fn list() -> Result<()> {
    let devices = Scope::list().context("USB enumeration failed")?;
    if devices.is_empty() {
        println!("No Hantek 1008 (USB 0783:5725) found.");
        if cfg!(target_os = "windows") {
            println!(
                "hint: plug it in; to be usable it must be bound to WinUSB \
                 (INF or Zadig) instead of Hantek's STM32MS driver."
            );
        } else if cfg!(target_os = "linux") {
            println!(
                "hint: plug it in and check `lsusb | grep 0783:5725`; \
                 install the udev rule so it is accessible without root."
            );
        }
        return Ok(());
    }
    for (i, d) in devices.iter().enumerate() {
        let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
        println!(
            "#{i}  bus {}  addr {}  product: {}  manufacturer: {}  serial: {}",
            opt(&d.bus),
            d.address.map_or_else(|| "-".into(), |a| a.to_string()),
            opt(&d.product),
            opt(&d.manufacturer),
            opt(&d.serial),
        );
        if let Some(driver) = &d.driver {
            let note = if driver.eq_ignore_ascii_case("winusb") {
                ""
            } else {
                "  (needs WinUSB to be usable)"
            };
            println!("    driver: {driver}{note}");
        }
    }
    Ok(())
}

fn identify(scope: &mut Scope) -> Result<()> {
    eprintln!("Initialising (takes a few seconds)...");
    let report = scope.init().context("init sequence failed")?;
    print_report(&report);
    Ok(())
}

fn print_report(r: &InitReport) {
    let [a, b] = r.firmware_version;
    println!("Firmware version : {a:02x} {b:02x}");
    let temp = u16::from_le_bytes(r.cal_temperature);
    println!(
        "Cal. temperature : {} (raw, little-endian = {temp})",
        hex(&r.cal_temperature)
    );
    println!("E6 readout       : {}", hex(&r.e6));
    println!();
    println!("Zero offsets (raw ADC counts, nominal 2048):");
    print!("  scale  ");
    for ch in 1..=CHANNELS {
        print!("{:>9}", format!("ch{ch}"));
    }
    println!();
    for scale in ScaleId::ALL {
        print!("  {:<7}", scale.volts_per_unit());
        for v in r.zero_offsets.for_scale(scale) {
            print!("{v:>9.1}");
        }
        println!();
    }
    println!();
    println!("Init blobs:");
    for (blob, data) in &r.blobs {
        println!("  {:02X} ({} bytes):", blob.id(), data.len());
        for line in data.chunks(16) {
            println!("    {}", hex(line));
        }
    }
}

fn capture(scope: &mut Scope, args: &CaptureArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    let channels = parse::parse_channels(&args.channels)?;
    let scales = parse::parse_scales(&args.scale, &channels)?;
    let (time_div, rounded) = parse::parse_time_div(&args.tdiv)?;
    if rounded {
        eprintln!(
            "note: {} is not a supported time/div; using {}",
            args.tdiv,
            parse::format_seconds(time_div.seconds_per_div())
        );
    }
    let trigger_ch = match args.trigger_ch {
        Some(ch) => *parse::parse_channels(&[ch])?
            .first()
            .context("trigger channel required")?,
        None => channels[0],
    };
    let mut enabled = [false; CHANNELS];
    for &ch in &channels {
        enabled[ch] = true;
    }
    let config = Config {
        channels: enabled,
        scales,
        time_div,
        trigger: Trigger {
            channel: trigger_ch as u8,
            slope: match args.slope {
                SlopeArg::Rising => Slope::Rising,
                SlopeArg::Falling => Slope::Falling,
            },
            level: args.level,
        },
    };
    config.validate()?;

    eprintln!("Initialising (takes a few seconds)...");
    scope.init_with(&config).context("init sequence failed")?;

    // Data arrives in ascending channel order regardless of --channels order.
    let active = config.active_channels();
    let sample_period = time_div.seconds_per_div() / SAMPLES_PER_DIV;
    let layout = csv::Layout {
        channels: &active,
        scales: &config.scales,
        sample_period,
        burst_column: args.count > 1,
    };
    let zero = *scope.zero_offsets();
    let nominal = usbscope_proto::ZeroOffsets::default();
    let values = if args.raw {
        csv::Values::Raw
    } else if args.measured_zero {
        csv::Values::Volts(&zero)
    } else {
        csv::Values::Volts(&nominal)
    };
    let trigger = if args.no_force {
        TriggerMode::Wait(Duration::from_millis(args.trigger_wait_ms))
    } else {
        TriggerMode::Force
    };

    let file =
        File::create(&args.out).with_context(|| format!("cannot create {}", args.out.display()))?;
    let mut out = BufWriter::new(file);
    csv::write_header(&mut out, &layout)?;
    let mut samples = 0;
    for burst in 0..args.count {
        let data = scope
            .capture_burst_with(trigger)
            .with_context(|| format!("burst {} of {} failed", burst + 1, args.count))?;
        samples = data.iter().map(Vec::len).min().unwrap_or(0);
        csv::write_burst(&mut out, &layout, &values, burst, &data)?;
    }
    out.flush()?;
    eprintln!(
        "Wrote {} burst(s) x {samples} samples/channel to {} \
         (time axis approximate: {} per sample, assuming {SAMPLES_PER_DIV} samples/div)",
        args.count,
        args.out.display(),
        parse::format_seconds(sample_period),
    );
    Ok(())
}

fn raw(scope: &mut Scope, args: &RawArgs) -> Result<()> {
    let bytes = parse::parse_hex_bytes(&args.bytes)?;
    let (&cmd_byte, params) = bytes.split_first().context("no command byte")?;
    let cmd = Command::Raw(RawCommand::new(
        cmd_byte,
        params,
        args.reply_len,
        !args.no_echo,
    )?);
    if args.init {
        eprintln!("Initialising (takes a few seconds)...");
        scope.init().context("init sequence failed")?;
    }
    println!("> {}", hex(&bytes));
    let reply = scope.transact_raw(&cmd)?;
    println!("< {} ({} bytes)", hex(&reply), reply.len());
    if !args.no_echo {
        match reply.first() {
            Some(&b) if b == cmd_byte => println!("echo: ok"),
            Some(&b) => println!("echo: MISMATCH (first byte {b:02x}, expected {cmd_byte:02x})"),
            None => println!("echo: none (empty reply)"),
        }
    }
    Ok(())
}

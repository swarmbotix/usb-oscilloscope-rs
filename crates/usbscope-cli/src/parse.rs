//! Argument parsing helpers (time/div strings, hex bytes, channel/scale lists).

use anyhow::{Context, Result, bail};
use usbscope_proto::{CHANNELS, ScaleId, TimeDivId};

/// Parse a duration like `1ms`, `500us`, `0.5ms`, `2ns`, `1s` or a bare
/// number of seconds.
pub fn parse_seconds(text: &str) -> Result<f64> {
    let t = text.trim();
    let (number, scale) = [
        ("ns", 1e-9),
        ("us", 1e-6),
        ("\u{b5}s", 1e-6),
        ("\u{3bc}s", 1e-6),
        ("ms", 1e-3),
        ("s", 1.0),
    ]
    .iter()
    .find_map(|(suffix, scale)| t.strip_suffix(suffix).map(|n| (n, *scale)))
    .unwrap_or((t, 1.0));
    let value: f64 = number
        .trim()
        .parse()
        .with_context(|| format!("invalid duration {text:?} (try 1ms, 500us, 2ns)"))?;
    if !value.is_finite() || value <= 0.0 {
        bail!("duration {text:?} must be positive");
    }
    Ok(value * scale)
}

/// Resolve a time/div string to the nearest supported ID; the flag is true
/// when it had to round.
pub fn parse_time_div(text: &str) -> Result<(TimeDivId, bool)> {
    let seconds = parse_seconds(text)?;
    let id = TimeDivId::from_seconds(seconds)
        .with_context(|| format!("no time/div matches {text:?}"))?;
    let exact = (id.seconds_per_div() - seconds).abs() <= seconds * 1e-6;
    Ok((id, !exact))
}

/// Human-readable seconds, e.g. `500us`, `2ms`.
pub fn format_seconds(seconds: f64) -> String {
    let (value, unit) = if seconds >= 1.0 {
        (seconds, "s")
    } else if seconds >= 1e-3 {
        (seconds * 1e3, "ms")
    } else if seconds >= 1e-6 {
        (seconds * 1e6, "us")
    } else {
        (seconds * 1e9, "ns")
    };
    format!("{}{unit}", trim_float(value))
}

fn trim_float(value: f64) -> String {
    let s = format!("{value:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Parse hex bytes from one or more arguments: `a5 5a`, `a55a`, `0xa5,0x5a`.
pub fn parse_hex_bytes(parts: &[String]) -> Result<Vec<u8>> {
    let mut digits = String::new();
    for part in parts {
        for token in part.split([' ', ',', ':', '-']) {
            let token = token.trim();
            let token = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .unwrap_or(token);
            if token.len() % 2 != 0 {
                bail!("hex token {token:?} has an odd number of digits");
            }
            digits.push_str(token);
        }
    }
    if digits.is_empty() {
        bail!("no bytes given");
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .with_context(|| format!("invalid hex byte {:?}", &digits[i..i + 2]))
        })
        .collect()
}

/// Validate 1-based channel numbers and return them deduplicated in the
/// order given.
pub fn parse_channels(channels: &[u8]) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for &ch in channels {
        if !(1..=CHANNELS as u8).contains(&ch) {
            bail!("channel {ch} out of range 1..={CHANNELS}");
        }
        let idx = usize::from(ch - 1);
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    if out.is_empty() {
        bail!("at least one channel is required");
    }
    Ok(out)
}

/// Build the per-channel scale array: one factor for all `channels`, or
/// one per channel in the order given. Unused channels get 1.0.
pub fn parse_scales(factors: &[f64], channels: &[usize]) -> Result<[ScaleId; CHANNELS]> {
    let lookup = |f: f64| {
        ScaleId::from_volts_per_unit(f)
            .with_context(|| format!("scale {f} not supported (use 1, 0.125 or 0.02)"))
    };
    let mut scales = [ScaleId::V1; CHANNELS];
    match factors {
        [one] => {
            let s = lookup(*one)?;
            for &ch in channels {
                scales[ch] = s;
            }
        }
        many if many.len() == channels.len() => {
            for (&ch, &f) in channels.iter().zip(many) {
                scales[ch] = lookup(f)?;
            }
        }
        many => bail!(
            "--scale has {} values but --channels has {}; give one value or one per channel",
            many.len(),
            channels.len()
        ),
    }
    Ok(scales)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds() {
        assert_eq!(parse_seconds("1ms").unwrap(), 1e-3);
        assert!((parse_seconds("500us").unwrap() - 5e-4).abs() < 1e-15);
        assert!((parse_seconds("500\u{b5}s").unwrap() - 5e-4).abs() < 1e-15);
        assert_eq!(parse_seconds("2ns").unwrap(), 2e-9);
        assert_eq!(parse_seconds("1s").unwrap(), 1.0);
        assert_eq!(parse_seconds("0.2").unwrap(), 0.2);
        assert!(parse_seconds("fast").is_err());
        assert!(parse_seconds("0ms").is_err());
        assert!(parse_seconds("-1ms").is_err());
    }

    #[test]
    fn time_div() {
        let (id, rounded) = parse_time_div("1ms").unwrap();
        assert_eq!((id.id(), rounded), (18, false));
        let (id, rounded) = parse_time_div("3ms").unwrap();
        assert_eq!(id.nanos_per_div(), 2_000_000);
        assert!(rounded);
        assert_eq!(format_seconds(5e-4), "500us");
        assert_eq!(format_seconds(0.2), "200ms");
        assert_eq!(format_seconds(1e-9), "1ns");
    }

    #[test]
    fn hex() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_hex_bytes(&args(&["a5", "5a"])).unwrap(), [0xa5, 0x5a]);
        assert_eq!(parse_hex_bytes(&args(&["a55a"])).unwrap(), [0xa5, 0x5a]);
        assert_eq!(
            parse_hex_bytes(&args(&["0xA5,0x5a"])).unwrap(),
            [0xa5, 0x5a]
        );
        assert_eq!(parse_hex_bytes(&args(&["e9"])).unwrap(), [0xe9]);
        assert!(parse_hex_bytes(&args(&["a5 5"])).is_err());
        assert!(parse_hex_bytes(&args(&["zz"])).is_err());
        assert!(parse_hex_bytes(&args(&[""])).is_err());
    }

    #[test]
    fn channels_and_scales() {
        assert_eq!(parse_channels(&[2, 1, 2]).unwrap(), [1, 0]);
        assert!(parse_channels(&[0]).is_err());
        assert!(parse_channels(&[9]).is_err());
        assert!(parse_channels(&[]).is_err());

        let s = parse_scales(&[0.02], &[0, 3]).unwrap();
        assert_eq!(s[0], ScaleId::V0_02);
        assert_eq!(s[3], ScaleId::V0_02);
        assert_eq!(s[1], ScaleId::V1);
        let s = parse_scales(&[0.125, 1.0], &[4, 2]).unwrap();
        assert_eq!(s[4], ScaleId::V0_125);
        assert_eq!(s[2], ScaleId::V1);
        assert!(parse_scales(&[0.5], &[0]).is_err());
        assert!(parse_scales(&[1.0, 1.0], &[0, 1, 2]).is_err());
    }
}

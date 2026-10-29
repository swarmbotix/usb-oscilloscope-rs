//! CSV output for burst captures.

use std::io::{self, Write};

use usbscope_proto::{ScaleId, ZeroOffsets, raw_to_volts};

/// How sample values are written.
pub enum Values<'a> {
    /// Raw ADC counts.
    Raw,
    /// Volts using these zero offsets.
    Volts(&'a ZeroOffsets),
}

/// Static description of the columns.
pub struct Layout<'a> {
    /// Zero-based channel index of each data column.
    pub channels: &'a [usize],
    /// Scale per channel index (all 8).
    pub scales: &'a [ScaleId],
    /// Seconds between consecutive samples of one channel.
    pub sample_period: f64,
    /// Prefix rows with a burst number (used when capturing several).
    pub burst_column: bool,
}

/// Write the header row: `[burst,]t_s,chN,...`.
pub fn write_header(out: &mut impl Write, layout: &Layout) -> io::Result<()> {
    if layout.burst_column {
        write!(out, "burst,")?;
    }
    write!(out, "t_s")?;
    for ch in layout.channels {
        write!(out, ",ch{}", ch + 1)?;
    }
    writeln!(out)
}

/// Write one burst; `data[i]` belongs to `layout.channels[i]`. Rows stop at
/// the shortest channel.
pub fn write_burst(
    out: &mut impl Write,
    layout: &Layout,
    values: &Values,
    burst: usize,
    data: &[Vec<u16>],
) -> io::Result<()> {
    let rows = data.iter().map(Vec::len).min().unwrap_or(0);
    for row in 0..rows {
        if layout.burst_column {
            write!(out, "{burst},")?;
        }
        write!(out, "{:.9}", row as f64 * layout.sample_period)?;
        for (col, &ch) in data.iter().zip(layout.channels) {
            let raw = col[row];
            match values {
                Values::Raw => write!(out, ",{raw}")?,
                Values::Volts(zero) => {
                    let scale = layout.scales[ch];
                    let volts = raw_to_volts(raw, zero.get(scale, ch), scale);
                    write!(out, ",{volts:.5}")?
                }
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volts_and_raw() {
        let scales = [ScaleId::V1; 8];
        let layout = Layout {
            channels: &[0, 2],
            scales: &scales,
            sample_period: 1e-3,
            burst_column: false,
        };
        let data = vec![vec![2048, 2148], vec![1948, 2048]];
        let zero = ZeroOffsets::default();

        let mut buf = Vec::new();
        write_header(&mut buf, &layout).unwrap();
        write_burst(&mut buf, &layout, &Values::Volts(&zero), 0, &data).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "t_s,ch1,ch3\n0.000000000,0.00000,-1.00000\n0.001000000,1.00000,0.00000\n"
        );

        let layout = Layout {
            burst_column: true,
            ..layout
        };
        let mut buf = Vec::new();
        write_header(&mut buf, &layout).unwrap();
        write_burst(&mut buf, &layout, &Values::Raw, 3, &data).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "burst,t_s,ch1,ch3\n3,0.000000000,2048,1948\n3,0.001000000,2148,2048\n"
        );
    }
}

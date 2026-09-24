//! Figures: the canvas a harness draws on, and the pieces its charts share.
//!
//! Images are PNG, drawn with `plotters` in the platform's own sans-serif font
//! (fontconfig on Linux, Core Text on macOS, DirectWrite on Windows). Time axes
//! are in milliseconds, and every chart on a canvas takes the same label widths
//! so their time axes line up.

use std::ops::Range;
use std::path::Path;

use anyhow::{Result, anyhow};
use plotters::coord::Shift;
use plotters::coord::types::RangedCoordf64;
use plotters::element::DashedPathElement;
use plotters::prelude::*;

/// The drawing surface of one image.
pub type Canvas<'a> = DrawingArea<BitMapBackend<'a>, Shift>;

/// A chart with linear axes on a [`Canvas`].
pub type Chart<'a, 'b> =
    ChartContext<'a, BitMapBackend<'b>, Cartesian2d<RangedCoordf64, RangedCoordf64>>;

/// The family every label is set in.
pub const FONT: &str = "sans-serif";
/// Text sizes, in pixels.
pub const TITLE: i32 = 34;
pub const SUBTITLE: i32 = 24;
pub const CAPTION: i32 = 26;
pub const LABEL: i32 = 20;

pub const INK: RGBColor = RGBColor(0x2C, 0x2C, 0x2C);
pub const WAVE: RGBColor = RGBColor(0x8A, 0x8A, 0x8A);

/// A white canvas of `size` pixels, written to `path` by [`DrawingArea::present`].
pub fn canvas(path: &Path, size: (u32, u32)) -> Result<Canvas<'_>> {
    let canvas = BitMapBackend::new(path, size).into_drawing_area();
    canvas.fill(&WHITE).map_err(draw_error)?;
    Ok(canvas)
}

/// Turns a `plotters` error into one `anyhow` can carry. Most are a missing
/// font or an unwritable path.
pub fn draw_error<E: std::fmt::Display>(e: E) -> anyhow::Error {
    anyhow!("drawing failed: {e}")
}

/// `sample` at `sample_rate`, in milliseconds.
pub fn ms(sample: usize, sample_rate: u32) -> f64 {
    sample as f64 * 1000.0 / sample_rate as f64
}

/// A level in dBFS, floored at `floor` so silence stays on the axis.
pub fn dbfs(level: f32, floor: f64) -> f64 {
    (20.0 * (level as f64).log10()).max(floor)
}

/// A captioned chart over `0..end` ms and `y`, its axes drawn. Only the bottom
/// chart of a figure takes `time_axis`, the "Time (ms)" description.
pub fn time_chart<'a, 'b>(
    area: &'a Canvas<'b>,
    caption: &str,
    end: f64,
    y: Range<f64>,
    y_desc: &str,
    y_decimals: usize,
    time_axis: bool,
) -> Result<Chart<'a, 'b>> {
    let mut chart = ChartBuilder::on(area)
        .caption(caption, (FONT, CAPTION))
        .margin(16)
        .x_label_area_size(if time_axis { 56 } else { 40 })
        .y_label_area_size(90)
        .build_cartesian_2d(0.0..end, y)
        .map_err(draw_error)?;
    let x_format = |v: &f64| format!("{v:.0}");
    let y_format = |v: &f64| format!("{v:.y_decimals$}");
    let mut mesh = chart.configure_mesh();
    mesh.disable_mesh()
        .y_desc(y_desc)
        .x_label_formatter(&x_format)
        .y_label_formatter(&y_format)
        .label_style((FONT, LABEL))
        .axis_desc_style((FONT, LABEL));
    if time_axis {
        mesh.x_desc("Time (ms)");
    }
    mesh.draw().map_err(draw_error)?;
    Ok(chart)
}

/// The chart's legend, top right on a pale backing.
pub fn legend<'a, 'b: 'a>(chart: &mut Chart<'a, 'b>) -> Result<()> {
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .label_font((FONT, LABEL))
        .background_style(WHITE.mix(0.9))
        .border_style(INK.mix(0.3))
        .draw()
        .map_err(draw_error)
}

/// A dashed segment from `from` to `to`: a threshold across a chart, or a marker
/// down one.
pub fn dashed(
    from: (f64, f64),
    to: (f64, f64),
    color: RGBColor,
) -> DashedLineSeries<std::array::IntoIter<(f64, f64), 2>, i32> {
    DashedLineSeries::new([from, to], 10, 6, color.stroke_width(2))
}

/// A legend swatch for a line.
pub fn line_swatch<C: Color + 'static>(color: C) -> impl Fn((i32, i32)) -> PathElement<(i32, i32)> {
    move |(x, y)| PathElement::new(vec![(x, y), (x + 24, y)], color.stroke_width(3))
}

/// A legend swatch for a [`dashed`] line.
pub fn dashed_swatch(
    color: RGBColor,
) -> impl Fn((i32, i32)) -> DashedPathElement<std::array::IntoIter<(i32, i32), 2>, i32> {
    move |(x, y)| DashedPathElement::new([(x, y), (x + 24, y)], 7, 4, color.stroke_width(2))
}

/// A legend swatch for a shaded region.
pub fn block_swatch(color: RGBAColor) -> impl Fn((i32, i32)) -> Rectangle<(i32, i32)> {
    move |(x, y)| Rectangle::new([(x, y - 7), (x + 24, y + 7)], color.filled())
}

/// Draws `audio` as its min–max envelope, one vertical stroke per pixel column,
/// over the chart's time range. At a few hundred samples per pixel a plain line
/// overdraws into a solid block without showing the envelope any better.
pub fn waveform(chart: &mut Chart, audio: &[f32], sample_rate: u32, color: RGBColor) -> Result<()> {
    let columns = chart.plotting_area().dim_in_pixel().0.max(1) as usize;
    let per_column = audio.len().div_ceil(columns).max(1);
    let strokes = audio.chunks(per_column).enumerate().map(|(i, chunk)| {
        let (lo, hi) = chunk
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        let t = ms(i * per_column, sample_rate);
        PathElement::new(vec![(t, lo as f64), (t, hi as f64)], color)
    });
    chart.draw_series(strokes).map_err(draw_error)?;
    Ok(())
}

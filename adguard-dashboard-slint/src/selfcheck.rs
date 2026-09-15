//! Offscreen render + pixel count: the Slint counterpart of the LVGL
//! dashboard's `--self-check`.
//!
//! It renders the dashboard with the software renderer into a plain buffer - no
//! display, no DRM lease - and counts pixels per known colour, so "is it drawn?"
//! is answered with numbers instead of eyes. Run it on the board (which has the
//! live AdGuard Home) or on the host pointed at one:
//!
//!     adguard-dashboard-slint --self-check [--config <path>] [--size WxH,WxH...]
//!
//! Every panel size it is given gets rendered and checked, because the layout is
//! derived from the panel (see ui/main.slint): the same component renders each
//! card in a row, so their boxes have to match, and the whole page has to stay
//! inside the buffer. A page laid out larger than the panel is the bug this
//! check exists to catch, and it is why the sizes are a parameter instead of a
//! constant: a check that only ever renders the canvas the markup was designed
//! on cannot see a panel that is smaller than that canvas.

use std::rc::Rc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::PhysicalSize;

use crate::agh::{self, FetchOutcome, Snapshot};
use crate::config::Config;
use crate::{view, MainWindow};

/// Rendered when `--size` says nothing else: the panels this app is deployed on
/// (the Pi 5 rig's 1366x768 monitor lives here) plus the two the design was
/// measured on.
const DEFAULT_SIZES: [(u32, u32); 3] = [(1366, 768), (1720, 1440), (1920, 1080)];

/// How close to the buffer's edge content may come before the page counts as
/// larger than the panel. The page keeps its padding there, so a few pixels is
/// all the slack a rounding error needs.
const EDGE_MARGIN: u32 = 4;

/// The card grid every panel has to produce: four charts, then two rows of two
/// tables. A row that is missing or holds the wrong number of cards means the
/// page did not lay out, whatever the pixel counts say.
const EXPECTED_CARDS: [usize; 3] = [4, 2, 2];

/// Opaque 32-bit pixel. Exact colours, which is what makes counting them simple.
#[derive(Clone, Copy, Default)]
struct Rgb32 {
    r: u8,
    g: u8,
    b: u8,
}

impl TargetPixel for Rgb32 {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        // Premultiplied source over an opaque destination.
        let inverse = 255 - color.alpha as u32;
        self.r = (color.red as u32 + self.r as u32 * inverse / 255) as u8;
        self.g = (color.green as u32 + self.g as u32 * inverse / 255) as u8;
        self.b = (color.blue as u32 + self.b as u32 * inverse / 255) as u8;
    }

    fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            r: red,
            g: green,
            b: blue,
        }
    }
}

/// "\"label\"=count" of the first row, for the self-check line.
fn first_of(rows: &[(String, u64)]) -> String {
    rows.first()
        .map(|(label, count)| format!("\"{label}\"={count}"))
        .unwrap_or_else(|| "(none)".to_string())
}

/// Panel sizes from `--size WxH[,WxH...]`, or `SLINT_SELFCHECK_SIZE`, or
/// `DEFAULT_SIZES`.
pub fn sizes_from_args(args: &[String]) -> Result<Vec<(u32, u32)>> {
    let spec = args
        .iter()
        .position(|arg| arg == "--size")
        .and_then(|index| args.get(index + 1).cloned())
        .or_else(|| std::env::var("SLINT_SELFCHECK_SIZE").ok());

    let Some(spec) = spec else {
        return Ok(DEFAULT_SIZES.to_vec());
    };

    let mut sizes = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        let (w, h) = part
            .split_once(['x', 'X'])
            .with_context(|| format!("--size {part}: expected WxH"))?;
        let size: (u32, u32) = (
            w.trim()
                .parse()
                .with_context(|| format!("--size {part}: width"))?,
            h.trim()
                .parse()
                .with_context(|| format!("--size {part}: height"))?,
        );
        if size.0 < 320 || size.1 < 240 {
            bail!("--size {part}: smaller than any panel this app runs on");
        }
        sizes.push(size);
    }
    if sizes.is_empty() {
        bail!("--size: no panel size given");
    }
    Ok(sizes)
}

/// Y positions of cell-text pixels and row separators within one column.
fn row_rhythm(
    buffer: &[Rgb32],
    width: u32,
    height: u32,
    x0: usize,
    x1: usize,
) -> (Vec<(u32, u32)>, Vec<u32>) {
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut separators: Vec<u32> = Vec::new();

    for y in 0..height as usize {
        let row = &buffer[(y * width as usize)..((y + 1) * width as usize)];
        let slice = &row[x0..x1.min(row.len())];
        if slice.iter().any(|p| (p.r, p.g, p.b) == view::GRID) {
            separators.push(y as u32);
        }
        if slice.iter().any(|p| (p.r, p.g, p.b) == view::TEXT) {
            match bands.last_mut() {
                Some(last) if last.1 + 1 == y as u32 => last.1 = y as u32,
                _ => bands.push((y as u32, y as u32)),
            }
        }
    }
    bands.truncate(20);
    separators.truncate(20);
    (bands, separators)
}

/// Bounding boxes of every card row on the page: the page colour separates the
/// cards horizontally and between rows, so a row of card colour yields one run
/// per card, and the band of rows containing card colour gives the vertical span.
fn card_rows(buffer: &[Rgb32], width: u32, height: u32) -> Vec<Vec<(u32, u32, u32, u32)>> {
    let is_page = |p: &Rgb32| (p.r, p.g, p.b) == view::PAGE;

    let mut band: Option<(u32, u32)> = None;
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for y in 0..height as usize {
        let row = &buffer[(y * width as usize)..((y + 1) * width as usize)];
        if row.iter().any(|p| (p.r, p.g, p.b) == view::CARD) {
            band = Some(match band {
                Some((y0, _)) => (y0, y as u32),
                None => (y as u32, y as u32),
            });
        } else if let Some(finished) = band.take() {
            bands.push(finished);
        }
    }
    if let Some(finished) = band {
        bands.push(finished);
    }

    bands
        .into_iter()
        .map(|(y0, y1)| {
            let y = (y0 + y1) / 2;
            let row =
                &buffer[((y as usize) * width as usize)..(((y as usize) + 1) * width as usize)];
            let mut runs: Vec<(u32, u32)> = Vec::new();
            let mut start: Option<u32> = None;
            for x in 0..width {
                if !is_page(&row[x as usize]) {
                    start.get_or_insert(x);
                } else if let Some(s) = start.take() {
                    runs.push((s, x - 1));
                }
            }
            if let Some(s) = start {
                runs.push((s, width - 1));
            }
            runs.into_iter().map(|(x0, x1)| (x0, x1, y0, y1)).collect()
        })
        .collect()
}

struct OffscreenPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

/// Render one panel and check it: the card grid, the row geometry, and that the
/// page stayed inside the buffer.
fn check_size(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
    expect_bars: bool,
) -> Result<()> {
    let start = Instant::now();
    window.set_size(PhysicalSize::new(width, height));
    window.request_redraw();

    let mut buffer = vec![Rgb32::default(); (width * height) as usize];
    let drew = window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, width as usize);
    });
    if !drew {
        bail!("{width}x{height}: the renderer reported nothing to draw");
    }
    println!(
        "[selfcheck] {width}x{height}: rendered in {} ms",
        start.elapsed().as_millis()
    );

    let mut page = 0u64;
    let mut card = 0u64;
    let mut badge = 0u64;
    let mut grid = 0u64;
    let mut bars = [0u64; view::SERIES.len()];
    for pixel in &buffer {
        let rgb = (pixel.r, pixel.g, pixel.b);
        if rgb == view::PAGE {
            page += 1;
        } else if rgb == view::CARD {
            card += 1;
        } else if rgb == view::GRID {
            grid += 1;
        } else if rgb == view::PROT_ON || rgb == view::PROT_OFF {
            badge += 1;
        } else {
            for (index, color) in view::SERIES.iter().enumerate() {
                if rgb == *color {
                    bars[index] += 1;
                }
            }
        }
    }

    println!(
        "[selfcheck] {width}x{height}: of {} pixels: page={page} card={card} grid={grid} badge={badge}",
        buffer.len()
    );
    for (index, count) in bars.iter().enumerate() {
        println!("[selfcheck] {width}x{height}: chart {index} bars px={count}");
    }

    // Row rhythm in the tables: text bands (bright cell text) and the row
    // separators in the label column, so a vertical layout problem shows up as
    // numbers instead of "the text looks top-aligned".
    let (bands, separators) = row_rhythm(&buffer, width, height, 28, 640);
    println!("[selfcheck] {width}x{height}: column x=28..640 text bands {bands:?}");
    println!("[selfcheck] {width}x{height}:                     separators {separators:?}");

    // Card geometry, row by row. One component renders each row's cards, so their
    // boxes have to match; the numbers say which card is off rather than "one of
    // them looks different". Widths may differ by a pixel: an odd remainder does
    // not divide between four cards.
    let rows = card_rows(&buffer, width, height);
    let counts: Vec<usize> = rows.iter().map(Vec::len).collect();
    for (index, row) in rows.iter().enumerate() {
        let boxes: Vec<String> = row
            .iter()
            .map(|(x0, x1, y0, y1)| {
                format!("{w}x{h} at ({x0},{y0})", w = x1 - x0 + 1, h = y1 - y0 + 1)
            })
            .collect();
        println!(
            "[selfcheck] {width}x{height}: card row {index}: {}",
            boxes.join(" | ")
        );
        let widths: Vec<u32> = row.iter().map(|(x0, x1, _, _)| x1 - x0 + 1).collect();
        let heights: Vec<u32> = row.iter().map(|(_, _, y0, y1)| y1 - y0 + 1).collect();
        if heights.windows(2).any(|pair| pair[0] != pair[1]) {
            bail!("{width}x{height}: card row {index} heights differ: {heights:?}");
        }
        if let (Some(min), Some(max)) = (widths.iter().min(), widths.iter().max()) {
            if max - min > 1 {
                bail!("{width}x{height}: card row {index} widths differ: {widths:?}");
            }
        }
    }
    if counts != EXPECTED_CARDS {
        bail!(
            "{width}x{height}: expected {EXPECTED_CARDS:?} cards per row, found {counts:?} \
             - a card row is missing or the page did not lay out"
        );
    }

    // The whole page has to be inside the panel: this is the check the layout
    // exists to satisfy, and the one that fails first if a fixed size creeps
    // back in.
    let (mut min_x, mut max_x) = (width as usize, 0usize);
    let (mut min_y, mut max_y) = (height as usize, 0usize);
    for y in 0..height as usize {
        let row = &buffer[(y * width as usize)..((y + 1) * width as usize)];
        for (x, pixel) in row.iter().enumerate() {
            if (pixel.r, pixel.g, pixel.b) != view::PAGE {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    println!(
        "[selfcheck] {width}x{height}: content x={min_x}..{max_x} y={min_y}..{max_y} of {width}x{height}"
    );
    if max_x as u32 + EDGE_MARGIN > width - 1 || max_y as u32 + EDGE_MARGIN > height - 1 {
        bail!(
            "{width}x{height}: the page is larger than the panel \
             (content reaches x={max_x} y={max_y}, buffer is {}x{})",
            width - 1,
            height - 1
        );
    }

    // A card background appearing at all means the layout and the painter ran;
    // without it something is fundamentally wrong (empty window, black screen).
    if card == 0 {
        bail!("{width}x{height}: no card-background pixels: nothing was laid out or painted");
    }
    if expect_bars && bars.iter().all(|count| *count == 0) {
        bail!("{width}x{height}: not one bar pixel: the charts have no room to draw in");
    }
    Ok(())
}

pub fn run(cfg: &Config, sizes: &[(u32, u32)]) -> Result<()> {
    let start = Instant::now();

    // Real data first: this exercises the API path as well as the rendering.
    let snapshot = match agh::fetch(cfg) {
        FetchOutcome::Ok(snapshot) => {
            println!(
                "[selfcheck] data: {} queries, {} blocked, {} buckets/series, version {} ({} ms)",
                snapshot.queries,
                snapshot.blocked_total(),
                snapshot.series[0].len(),
                if snapshot.version.is_empty() {
                    "?"
                } else {
                    &snapshot.version
                },
                start.elapsed().as_millis()
            );
            // The top lists are the part of the API that changed shape between
            // AdGuard builds: show what was parsed, not just that it was.
            println!(
                "[selfcheck] top: clients={} {} queried={} {} blocked={} {}",
                snapshot.top_clients.len(),
                first_of(&snapshot.top_clients),
                snapshot.top_queried.len(),
                first_of(&snapshot.top_queried),
                snapshot.top_blocked.len(),
                first_of(&snapshot.top_blocked),
            );
            if !snapshot.top_clients.is_empty()
                && snapshot
                    .top_clients
                    .iter()
                    .all(|(label, _)| label.is_empty())
            {
                bail!("top-list labels parsed empty: the API shape changed again");
            }
            snapshot
        }
        FetchOutcome::CredentialsRejected => {
            bail!("credentials rejected (HTTP 401) - check the config")
        }
        FetchOutcome::Failed(msg) => {
            println!("[selfcheck] data unavailable: {msg} - rendering an empty snapshot");
            Snapshot::default()
        }
    };

    // A full repaint per size: with a reused buffer the renderer is free to draw
    // only the damaged region, and a fresh buffer for a new size would then come
    // back partly empty and measure as a broken layout.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(OffscreenPlatform {
        window: window.clone(),
    }))
    .map_err(|err| anyhow::anyhow!("installing the offscreen platform: {err}"))?;

    crate::register_font();

    let ui = MainWindow::new().context("creating the UI")?;
    view::fill(&ui, &snapshot, &cfg.base_url);

    let expect_bars = snapshot
        .series
        .first()
        .is_some_and(|series| !series.is_empty());
    for &(width, height) in sizes {
        check_size(&window, width, height, expect_bars)
            .with_context(|| format!("panel {width}x{height}"))?;
    }
    println!("[selfcheck] {} panel size(s) ok", sizes.len());
    Ok(())
}

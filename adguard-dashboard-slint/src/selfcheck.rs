//! Offscreen render + pixel count: the Slint counterpart of the LVGL
//! dashboard's `--self-check`.
//!
//! It renders the dashboard with the software renderer into a plain buffer - no
//! display, no DRM lease - and counts pixels per known colour, so "is it drawn?"
//! is answered with numbers instead of eyes. Run it on the board (which has the
//! live AdGuard Home) or on the host pointed at one:

//!     adguard-dashboard-slint --self-check [--config <path>]

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

/// The board's panel: the same geometry the linuxsgc backend would get.
const WIDTH: u32 = 1720;
const HEIGHT: u32 = 1440;

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

/// Y positions of cell-text pixels and row separators within one column.
fn row_rhythm(buffer: &[Rgb32], x0: usize, x1: usize) -> (Vec<(u32, u32)>, Vec<u32>) {
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut separators: Vec<u32> = Vec::new();

    for y in 0..HEIGHT {
        let row = &buffer[(y * WIDTH) as usize..((y + 1) * WIDTH) as usize];
        let slice = &row[x0..x1];
        if slice.iter().any(|p| (p.r, p.g, p.b) == crate::view::GRID) {
            separators.push(y);
        }
        if slice.iter().any(|p| (p.r, p.g, p.b) == crate::view::TEXT) {
            match bands.last_mut() {
                Some(last) if last.1 + 1 == y => last.1 = y,
                _ => bands.push((y, y)),
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
///
/// Cards rendered from one component must come out identical; anything else is a
/// layout bug, not a rendering one, so the caller fails on a mismatch.
fn card_rows(buffer: &[Rgb32]) -> Vec<Vec<(u32, u32, u32, u32)>> {
    let is_page = |p: &Rgb32| (p.r, p.g, p.b) == view::PAGE;

    let mut band: Option<(u32, u32)> = None;
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for y in 0..HEIGHT {
        let row = &buffer[(y * WIDTH) as usize..((y + 1) * WIDTH) as usize];
        if row.iter().any(|p| (p.r, p.g, p.b) == view::CARD) {
            band = Some(match band {
                Some((y0, _)) => (y0, y),
                None => (y, y),
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
            let row = &buffer[(y * WIDTH) as usize..((y + 1) * WIDTH) as usize];
            let mut runs: Vec<(u32, u32)> = Vec::new();
            let mut start: Option<u32> = None;
            for x in 0..WIDTH {
                if !is_page(&row[x as usize]) {
                    start.get_or_insert(x);
                } else if let Some(s) = start.take() {
                    runs.push((s, x - 1));
                }
            }
            if let Some(s) = start {
                runs.push((s, WIDTH - 1));
            }
            runs.into_iter()
                .map(|(x0, x1)| (x0, x1, y0, y1))
                .collect()
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

pub fn run(cfg: &Config) -> Result<()> {
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
                && snapshot.top_clients.iter().all(|(label, _)| label.is_empty())
            {
                bail!("top-list labels parsed empty: the API shape changed again");
            }
            snapshot
        }
        FetchOutcome::CredentialsRejected => bail!("credentials rejected (HTTP 401) - check the config"),
        FetchOutcome::Failed(msg) => {
            println!("[selfcheck] data unavailable: {msg} - rendering an empty snapshot");
            Snapshot::default()
        }
    };

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(OffscreenPlatform {
        window: window.clone(),
    }))
    .map_err(|err| anyhow::anyhow!("installing the offscreen platform: {err}"))?;

    crate::register_font();

    let ui = MainWindow::new().context("creating the UI")?;
    view::fill(&ui, &snapshot, &cfg.base_url);
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));

    let mut buffer = vec![Rgb32::default(); (WIDTH * HEIGHT) as usize];
    let drew = window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, WIDTH as usize);
        println!(
            "[selfcheck] rendered {}x{} in {} ms",
            WIDTH,
            HEIGHT,
            start.elapsed().as_millis()
        );
    });
    if !drew {
        bail!("the renderer reported nothing to draw");
    }

    let mut page = 0u64;
    let mut card = 0u64;
    let mut badge = 0u64;
    let mut bars = [0u64; 4];
    let mut grid = 0u64;
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
        "[selfcheck] of {} pixels: page={page} card={card} grid={grid} badge={badge}",
        buffer.len()
    );
    for (index, count) in bars.iter().enumerate() {
        println!("[selfcheck] chart {index} bars px={count}");
    }

    // Row rhythm in the tables: text bands (bright cell text) and the row
    // separators in the label column, so a vertical layout problem shows up as
    // numbers instead of "the text looks top-aligned".
    let (bands, separators) = row_rhythm(&buffer, 28, 640);
    println!("[selfcheck] column x=28..640 text bands {bands:?}");
    println!("[selfcheck]                     separators {separators:?}");

    // Card geometry, row by row: one component renders each row's cards, so their
    // boxes have to match. A mismatch is a layout bug, and the numbers say which
    // card is off rather than "one of them looks different".
    let rows = card_rows(&buffer);
    for (index, row) in rows.iter().enumerate() {
        let boxes: Vec<String> = row
            .iter()
            .map(|(x0, x1, y0, y1)| {
                format!(
                    "{w}x{h} at ({x0},{y0})",
                    w = x1 - x0 + 1,
                    h = y1 - y0 + 1
                )
            })
            .collect();
        println!("[selfcheck] card row {index}: {}", boxes.join(" | "));
        let widths: Vec<u32> = row.iter().map(|(x0, x1, _, _)| x1 - x0 + 1).collect();
        let heights: Vec<u32> = row.iter().map(|(_, _, y0, y1)| y1 - y0 + 1).collect();
        if widths.windows(2).any(|pair| pair[0] != pair[1])
            || heights.windows(2).any(|pair| pair[0] != pair[1])
        {
            bail!("card row {index} is not equal: widths {widths:?}, heights {heights:?}");
        }
    }

    // A card background appearing at all means the layout and the painter ran;
    // without it something is fundamentally wrong (empty window, black screen).
    if card == 0 {
        bail!("no card-background pixels: nothing was laid out or painted");
    }
    Ok(())
}

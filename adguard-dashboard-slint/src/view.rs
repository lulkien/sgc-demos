//! Snapshot -> UI: the palette and the model building, shared by the live app
//! and the offscreen self-check. The colours mirror the LVGL dashboard, and
//! they are plain `(r, g, b)` triples so the self-check can count them in the
//! rendered buffer without going through Slint's colour type.

use slint::{Color, ModelRc, VecModel};

use crate::agh::Snapshot;
use crate::{ChartData, MainWindow, RowData, TableData};

pub const PAGE: (u8, u8, u8) = (0x0F, 0x14, 0x19);
pub const CARD: (u8, u8, u8) = (0x19, 0x21, 0x2A);
pub const GRID: (u8, u8, u8) = (0x2B, 0x36, 0x44);
pub const TEXT: (u8, u8, u8) = (0xE6, 0xED, 0xF3);
pub const MUTED: (u8, u8, u8) = (0x8B, 0x98, 0xA5);
pub const BAD: (u8, u8, u8) = (0xE5, 0x53, 0x4B);
pub const PROT_ON: (u8, u8, u8) = (0x2F, 0x9E, 0x63);
pub const PROT_OFF: (u8, u8, u8) = (0xD5, 0x47, 0x3F);

/// One per chart card, in the order the row shows them.
pub const SERIES: [(u8, u8, u8); 4] = [
    (0x4F, 0xA3, 0xE3),
    (0xE8, 0x84, 0x3C),
    (0xA8, 0x6F, 0xE0),
    (0x4F, 0xD1, 0xA5),
];
const CHART_TITLES: [&str; 4] = [
    "DNS queries",
    "Blocked by filters",
    "Blocked malware / phishing",
    "Blocked adult websites",
];

fn color(rgb: (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(rgb.0, rgb.1, rgb.2)
}

/// Push a snapshot into the UI: charts, tables, badge, footer.
pub fn fill(ui: &MainWindow, snapshot: &Snapshot, source: &str) {
    ui.set_footer_source(source.into());
    ui.set_footer_version(
        format!(
            "AdGuard Home {}",
            if snapshot.version.is_empty() {
                "version unknown"
            } else {
                &snapshot.version
            }
        )
        .into(),
    );

    ui.set_protection_known(snapshot.protection.is_some());
    ui.set_protection_on(snapshot.protection.unwrap_or(false));
    ui.set_protection_text(
        match snapshot.protection {
            Some(true) => "Protection: enabled",
            Some(false) => "Protection: disabled",
            None => "Protection: unknown",
        }
        .into(),
    );

    // Chart totals are the running counters (what the general table shows), not
    // the sum of the charted window - same as the LVGL dashboard.
    let totals = [
        snapshot.queries,
        snapshot.blocked_filtering,
        snapshot.safebrowsing,
        snapshot.parental,
    ];
    let charts: Vec<ChartData> = (0..CHART_TITLES.len())
        .map(|index| {
            let values = snapshot.series.get(index).map(Vec::as_slice).unwrap_or(&[]);
            let max = values.iter().copied().max().unwrap_or(0);
            ChartData {
                title: CHART_TITLES[index].into(),
                total: format!("{}", totals[index]).into(),
                color: color(SERIES[index]),
                values: ModelRc::new(VecModel::from(normalise(values, max))),
                empty: max == 0,
            }
        })
        .collect();
    ui.set_charts(ModelRc::new(VecModel::from(charts)));

    ui.set_general(table(
        "General statistics",
        "Metric",
        "Value",
        vec![
            row("DNS queries", &snapshot.queries.to_string()),
            row("Blocked by filters", &snapshot.blocked_filtering.to_string()),
            row("Blocked malware / phishing", &snapshot.safebrowsing.to_string()),
            row("Blocked adult websites", &snapshot.parental.to_string()),
            row("Blocked safe search", &snapshot.safesearch.to_string()),
            row(
                "Blocked total",
                &format!(
                    "{} ({:.1}%)",
                    snapshot.blocked_total(),
                    snapshot.blocked_percent()
                ),
            ),
            row(
                "Average processing time",
                &format!("{:.1} ms", snapshot.avg_processing_ms),
            ),
        ],
    ));
    ui.set_top_clients(list("Top clients", "Client", "Requests", &snapshot.top_clients));
    ui.set_top_queried(list(
        "Top queried domains",
        "Domain",
        "Requests",
        &snapshot.top_queried,
    ));
    ui.set_top_blocked(list(
        "Top blocked domains",
        "Domain",
        "Requests",
        &snapshot.top_blocked,
    ));

    ui.set_status_color(color(MUTED));
    ui.set_status_text("updated 0s ago".into());
}

/// Report the age of the data, once per UI tick.
pub fn set_age(ui: &MainWindow, age_secs: f64, stale_after: f64) {
    if age_secs > stale_after {
        ui.set_status_color(color(BAD));
        ui.set_status_text("stale".into());
    } else {
        ui.set_status_color(color(MUTED));
        ui.set_status_text(format!("updated {}s ago", age_secs as u64).into());
    }
}

pub fn set_error(ui: &MainWindow, text: &str) {
    ui.set_status_color(color(BAD));
    ui.set_status_text(text.into());
}

/// Normalise a series to 0..1 by its own maximum (the bar heights).
fn normalise(values: &[u32], max: u32) -> Vec<f32> {
    if max == 0 {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|value| *value as f32 / max as f32)
        .collect()
}

fn row(label: &str, value: &str) -> RowData {
    RowData {
        label: label.into(),
        value: value.into(),
    }
}

fn table(title: &str, col0: &str, col1: &str, rows: Vec<RowData>) -> TableData {
    TableData {
        title: title.into(),
        col0: col0.into(),
        col1: col1.into(),
        rows: ModelRc::new(VecModel::from(rows)),
    }
}

fn list(title: &str, col0: &str, col1: &str, entries: &[(String, u64)]) -> TableData {
    table(
        title,
        col0,
        col1,
        entries
            .iter()
            .map(|(label, count)| row(label, &count.to_string()))
            .collect(),
    )
}

//! AdGuard Home API client for `/control/stats` and `/control/status`.
//!
//! Two rules carried over from the LVGL dashboard:
//!
//! * the per-bucket series arrays cover the server's **whole statistics
//!   interval, oldest first** (a 7-day interval arrives as 168 hourly buckets),
//!   so only the newest [`CHART_BUCKETS`] are charted - charting the head plots
//!   the oldest, mostly idle hours and the charts look empty;
//! * a failed refresh keeps the previous snapshot, so the screen never blanks.

use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine;
use serde::Deserialize;

use crate::config::Config;

/// Buckets charted per series (the newest ones).
pub const CHART_BUCKETS: usize = 24;
/// Rows shown per top-list table.
pub const TOP_ROWS: usize = 9;

/// Why a refresh attempt ended the way it did; the UI shows this in the header.
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Ok(Snapshot),
    /// HTTP 401: the credentials in the config are wrong.
    CredentialsRejected,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub queries: u64,
    pub blocked_filtering: u64,
    pub safebrowsing: u64,
    pub parental: u64,
    pub safesearch: u64,
    pub avg_processing_ms: f64,
    pub series: [Vec<u32>; 4],
    pub top_clients: Vec<(String, u64)>,
    pub top_queried: Vec<(String, u64)>,
    pub top_blocked: Vec<(String, u64)>,
    pub protection: Option<bool>,
    pub version: String,
    pub at: Option<Instant>,
}

impl Snapshot {
    pub fn blocked_total(&self) -> u64 {
        self.blocked_filtering + self.safebrowsing + self.parental
    }

    pub fn blocked_percent(&self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            self.blocked_total() as f64 * 100.0 / self.queries as f64
        }
    }

    /// Seconds since this snapshot was fetched.
    pub fn age_secs(&self) -> f64 {
        self.at.map(|at| at.elapsed().as_secs_f64()).unwrap_or(f64::MAX)
    }
}

/// Fetch both endpoints and build a snapshot. `now` is stamped on success so the
/// caller can report the age of the data.
pub fn fetch(cfg: &Config) -> FetchOutcome {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(cfg.timeout_ms))
        .build();
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", cfg.username, cfg.password))
    );
    let base = cfg.base_url.trim_end_matches('/');

    let stats_text = match get(&agent, base, "/control/stats", &auth) {
        Ok(text) => text,
        Err(Outcome::Credentials) => return FetchOutcome::CredentialsRejected,
        Err(Outcome::Other(msg)) => return FetchOutcome::Failed(format!("/control/stats: {msg}")),
    };

    let stats: Stats = match serde_json::from_str(&stats_text) {
        Ok(stats) => stats,
        Err(err) => return FetchOutcome::Failed(format!("parsing /control/stats: {err}")),
    };

    // The status endpoint is secondary: a failure leaves the badge unknown
    // rather than blanking the dashboard.
    let status = match get(&agent, base, "/control/status", &auth) {
        Ok(text) => serde_json::from_str::<Status>(&text).ok(),
        Err(Outcome::Credentials) => return FetchOutcome::CredentialsRejected,
        Err(Outcome::Other(_)) => None,
    };

    let status = status.unwrap_or_default();
    FetchOutcome::Ok(Snapshot {
        queries: stats.num_dns_queries,
        blocked_filtering: stats.num_blocked_filtering,
        safebrowsing: stats.num_replaced_safebrowsing,
        parental: stats.num_replaced_parental,
        safesearch: stats.num_replaced_safesearch,
        avg_processing_ms: stats.avg_processing_time * 1000.0,
        series: [
            newest(&stats.dns_queries),
            newest(&stats.blocked_filtering),
            newest(&stats.replaced_safebrowsing),
            newest(&stats.replaced_parental),
        ],
        top_clients: entries(&stats.top_clients),
        top_queried: entries(&stats.top_queried_domains),
        top_blocked: entries(&stats.top_blocked_domains),
        protection: status.protection_enabled,
        version: status.version,
        at: Some(Instant::now()),
    })
}

enum Outcome {
    Credentials,
    Other(String),
}

fn get(agent: &ureq::Agent, base: &str, path: &str, auth: &str) -> Result<String, Outcome> {
    let url = format!("{base}{path}");
    let response = agent
        .get(&url)
        .set("Authorization", auth)
        .set("User-Agent", "agh-dashboard-slint")
        .call();

    match response {
        Ok(response) => response
            .into_string()
            .map_err(|err| Outcome::Other(format!("reading body: {err}"))),
        Err(ureq::Error::Status(401, _)) => Err(Outcome::Credentials),
        Err(ureq::Error::Status(code, _)) => Err(Outcome::Other(format!("HTTP {code}"))),
        Err(ureq::Error::Transport(err)) => Err(Outcome::Other(format!("{err}"))),
    }
}

/// The newest `CHART_BUCKETS` values of a series that arrives oldest-first.
fn newest(values: &[u32]) -> Vec<u32> {
    let start = values.len().saturating_sub(CHART_BUCKETS);
    values[start..].to_vec()
}

/// Top-list rows for the tables, as (label, count).
///
/// AdGuard Home returns these lists as single-key objects - the domain or client
/// address is the key, the count the value: `{"pass.proton.me": 23}`. Older
/// builds used explicit fields (`{"domain": ..., "count": ...}`,
/// `{"name": ..., "count": ...}`), so both shapes are accepted. An unrecognised
/// entry yields no row rather than an empty label with a zero count.
fn entries(list: &[serde_json::Value]) -> Vec<(String, u64)> {
    list.iter().filter_map(entry_row).take(TOP_ROWS).collect()
}

fn entry_row(entry: &serde_json::Value) -> Option<(String, u64)> {
    let map = entry.as_object()?;

    if map.len() == 1 {
        let (label, value) = map.iter().next()?;
        if let Some(count) = as_count(value) {
            return Some((label.clone(), count));
        }
    }

    let label = ["domain", "name", "ip"]
        .iter()
        .find_map(|key| map.get(*key).and_then(|value| value.as_str()))
        .filter(|label| !label.is_empty())?;
    Some((label.to_string(), map.get("count").and_then(as_count).unwrap_or(0)))
}

fn as_count(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

#[derive(Debug, Default, Deserialize)]
struct Stats {
    #[serde(default)]
    num_dns_queries: u64,
    #[serde(default)]
    num_blocked_filtering: u64,
    #[serde(default)]
    num_replaced_safebrowsing: u64,
    #[serde(default)]
    num_replaced_parental: u64,
    #[serde(default)]
    num_replaced_safesearch: u64,
    #[serde(default)]
    avg_processing_time: f64,
    #[serde(default)]
    dns_queries: Vec<u32>,
    #[serde(default)]
    blocked_filtering: Vec<u32>,
    #[serde(default)]
    replaced_safebrowsing: Vec<u32>,
    #[serde(default)]
    replaced_parental: Vec<u32>,
    #[serde(default)]
    top_queried_domains: Vec<serde_json::Value>,
    #[serde(default)]
    top_blocked_domains: Vec<serde_json::Value>,
    #[serde(default)]
    top_clients: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct Status {
    #[serde(default)]
    version: String,
    #[serde(default)]
    protection_enabled: Option<bool>,
}

/// Used by tests and the self-check to keep the "newest" rule honest.
#[cfg(test)]
mod tests {
    use super::newest;

    #[test]
    fn newest_keeps_the_tail() {
        let values: Vec<u32> = (1..=30).collect();
        let kept = newest(&values);
        assert_eq!(kept.len(), super::CHART_BUCKETS);
        assert_eq!(kept.last().copied(), Some(30));
        assert_eq!(kept.first().copied(), Some(7));
        assert!(newest(&[]).is_empty());
    }
}

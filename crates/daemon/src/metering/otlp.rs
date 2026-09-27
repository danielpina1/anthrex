//! OTLP/HTTP-JSON metrics into usage per `(run, role)` (M8b decision 30). Pure — no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`
//! (design decision 1).
//!
//! The body is untrusted: any local process can post to the loopback port. Parsing
//! never panics, every number is clamped, and the ledger is bounded ([`MAX_SERIES`],
//! [`MAX_TOTALS`]), so a hostile body costs at most its own size once.

use std::collections::{BTreeMap, BTreeSet};

use proto::TokenUsage;

/// The one metric read (M8b.1 item 5).
pub const TOKEN_METRIC: &str = "claude_code.token.usage";
/// Cumulative series the ledger remembers; a point of a new series beyond it is dropped.
pub const MAX_SERIES: usize = 4096;
/// `(run, role)` totals the ledger keeps; a point for a new pair beyond it is dropped.
pub const MAX_TOTALS: usize = 1024;
/// The longest run id, role, session id or model a point may carry.
pub const MAX_ID_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UsageKind {
    Input,
    Output,
    CacheRead,
    CacheWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagePoint {
    pub run_id: String,
    pub role: String,
    pub session_id: String,
    pub model: String,
    pub kind: UsageKind,
    pub value: u64,
    pub cumulative: bool,
}

/// Every sum data point of [`TOKEN_METRIC`] whose resource names `anthrex.run`.
///
/// Only a body that is not JSON, or not shaped as an export request, is an error.
/// A point this cannot use — no run, an id over [`MAX_ID_BYTES`], an unknown `type`,
/// an unspecified temporality, a value that is negative, not finite or not a number —
/// is dropped. Values are in `asDouble` (M8b.1) or `asInt`, a number or protobuf-JSON's
/// decimal string.
pub fn parse_metrics(body: &[u8]) -> Result<Vec<UsagePoint>, String> {
    // serde's derived structs also accept an array; an export request is an object.
    if body.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err("not an OTLP JSON export: the body is not a JSON object".to_string());
    }
    let request: wire::Request =
        serde_json::from_slice(body).map_err(|e| format!("not an OTLP JSON export: {e}"))?;
    let mut points = Vec::new();
    for resource in &request.resource_metrics {
        let attrs = &resource.resource.attributes;
        let Some(run_id) = wire::attribute(attrs, "anthrex.run").filter(|r| !r.is_empty()) else {
            continue;
        };
        let role = wire::attribute(attrs, "anthrex.role").unwrap_or_default();
        for metric in resource.scope_metrics.iter().flat_map(|s| &s.metrics) {
            let (true, Some(sum)) = (metric.name == TOKEN_METRIC, &metric.sum) else {
                continue;
            };
            let cumulative = match sum
                .aggregation_temporality
                .as_ref()
                .and_then(wire::temporality)
            {
                Some(1) => false,
                Some(2) => true,
                _ => continue,
            };
            for point in &sum.data_points {
                let attrs = &point.attributes;
                let kind = match wire::attribute(attrs, "type").as_deref() {
                    Some("input") => UsageKind::Input,
                    Some("output") => UsageKind::Output,
                    Some("cacheRead") => UsageKind::CacheRead,
                    Some("cacheCreation") => UsageKind::CacheWrite,
                    _ => continue,
                };
                let Some(value) = point.value() else { continue };
                let point = UsagePoint {
                    run_id: run_id.clone(),
                    role: role.clone(),
                    session_id: wire::attribute(attrs, "session.id").unwrap_or_default(),
                    model: wire::attribute(attrs, "model").unwrap_or_default(),
                    kind,
                    value,
                    cumulative,
                };
                let ids = [&point.run_id, &point.role, &point.session_id, &point.model];
                if ids.iter().all(|id| id.len() <= MAX_ID_BYTES) {
                    points.push(point);
                }
            }
        }
    }
    Ok(points)
}

/// A cumulative series: `(run, role, session.id, model, type)`.
type Series = (String, String, String, String, UsageKind);

/// Totals per `(run, role)`, and the last value of every cumulative series.
#[derive(Debug, Default)]
pub struct OtlpLedger {
    last: BTreeMap<Series, u64>,
    totals: BTreeMap<(String, String), TokenUsage>,
}

impl OtlpLedger {
    /// Adds `points`: a delta point's value; a cumulative point's increase over its
    /// series' last value, or its value when the series went down (a reset). Returns the
    /// `(run, role)` pairs they touched, in order. A point for a new pair or a new series
    /// beyond the caps is dropped; every sum saturates.
    pub fn apply(&mut self, points: &[UsagePoint]) -> Vec<(String, String)> {
        let mut touched = BTreeSet::new();
        for point in points {
            let pair = (point.run_id.clone(), point.role.clone());
            if !self.totals.contains_key(&pair) && self.totals.len() >= MAX_TOTALS {
                continue;
            }
            let added = if point.cumulative {
                let series = (
                    point.run_id.clone(),
                    point.role.clone(),
                    point.session_id.clone(),
                    point.model.clone(),
                    point.kind,
                );
                let last = match self.last.get(&series) {
                    Some(last) => Some(*last),
                    None if self.last.len() >= MAX_SERIES => continue,
                    None => None,
                };
                self.last.insert(series, point.value);
                match last {
                    Some(last) if point.value >= last => point.value - last,
                    _ => point.value,
                }
            } else {
                point.value
            };
            let total = self.totals.entry(pair.clone()).or_default();
            let field = match point.kind {
                UsageKind::Input => &mut total.input,
                UsageKind::Output => &mut total.output,
                UsageKind::CacheRead => &mut total.cache_read,
                UsageKind::CacheWrite => &mut total.cache_write,
            };
            *field = field.saturating_add(added);
            touched.insert(pair);
        }
        touched.into_iter().collect()
    }

    pub fn total(&self, run_id: &str, role: &str) -> TokenUsage {
        self.totals
            .get(&(run_id.to_string(), role.to_string()))
            .copied()
            .unwrap_or_default()
    }
}

/// The variables milestone 9 gives the orchestrator's window (M8b.1 item 5 fixed the
/// list: the documented set, no compression setting).
pub fn orchestrator_env(addr: &str, run_id: &str) -> Vec<(String, String)> {
    [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1".to_string()),
        ("OTEL_METRICS_EXPORTER", "otlp".to_string()),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json".to_string()),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", addr.to_string()),
        ("OTEL_METRIC_EXPORT_INTERVAL", "1000".to_string()),
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            format!("anthrex.run={run_id},anthrex.role=orchestrator"),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The part of `ExportMetricsServiceRequest`'s JSON form this reads. Unknown fields
/// are skipped without being kept.
mod wire {
    use serde::Deserialize;

    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Request {
        pub resource_metrics: Vec<ResourceMetrics>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct ResourceMetrics {
        pub resource: Resource,
        pub scope_metrics: Vec<ScopeMetrics>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    pub(super) struct Resource {
        pub attributes: Vec<KeyValue>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    pub(super) struct ScopeMetrics {
        pub metrics: Vec<Metric>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    pub(super) struct Metric {
        pub name: String,
        pub sum: Option<Sum>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Sum {
        pub data_points: Vec<DataPoint>,
        pub aggregation_temporality: Option<Scalar>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct DataPoint {
        pub attributes: Vec<KeyValue>,
        pub as_double: Option<Scalar>,
        pub as_int: Option<Scalar>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    pub(super) struct KeyValue {
        pub key: String,
        pub value: AnyValue,
    }

    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct AnyValue {
        pub string_value: Option<String>,
    }

    /// A number, or protobuf-JSON's string form of one (`"12"`, `"NaN"`, an enum name).
    #[derive(Deserialize)]
    #[serde(untagged)]
    pub(super) enum Scalar {
        Number(serde_json::Number),
        Text(String),
    }

    impl DataPoint {
        /// The value as a whole count; `None` when negative, not finite or not a number.
        pub fn value(&self) -> Option<u64> {
            let number = match self.as_double.as_ref().or(self.as_int.as_ref())? {
                Scalar::Number(n) => match n.as_u64() {
                    Some(exact) => return Some(exact),
                    None => n.as_f64()?,
                },
                Scalar::Text(t) => match t.trim().parse::<u64>() {
                    Ok(exact) => return Some(exact),
                    Err(_) => t.trim().parse::<f64>().ok()?,
                },
            };
            // `as` saturates at `u64::MAX`; NaN and negatives are refused first.
            (number.is_finite() && number >= 0.0).then_some(number as u64)
        }
    }

    /// `1` delta, `2` cumulative, from the number or the enum's name.
    pub(super) fn temporality(value: &Scalar) -> Option<u64> {
        match value {
            Scalar::Number(n) => n.as_u64(),
            Scalar::Text(t) => match t.as_str() {
                "AGGREGATION_TEMPORALITY_DELTA" | "1" => Some(1),
                "AGGREGATION_TEMPORALITY_CUMULATIVE" | "2" => Some(2),
                _ => None,
            },
        }
    }

    /// The first string attribute named `key`.
    pub(super) fn attribute(attrs: &[KeyValue], key: &str) -> Option<String> {
        attrs
            .iter()
            .find(|a| a.key == key)
            .and_then(|a| a.value.string_value.clone())
    }
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;

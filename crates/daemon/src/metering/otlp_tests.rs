//! M8b.15: `parse_metrics`, the ledger and `orchestrator_env`.

use proto::TokenUsage;
use serde_json::{Value, json};

use super::*;

/// M8b.1 item 5's recorded body (`"observed": true` in its meta file).
const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/otlp/claude-2.1.280-metrics.json");

fn attr(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

/// One `claude_code.token.usage` sum with `points` as `(type, value)`, under a resource
/// with `resource` attributes.
fn body(resource: &[(&str, &str)], temporality: u8, points: &[(&str, Value)]) -> Vec<u8> {
    let points: Vec<Value> = points
        .iter()
        .map(|(kind, value)| {
            json!({
                "attributes": [attr("type", kind), attr("session.id", "s1"), attr("model", "m1")],
                "asDouble": value,
            })
        })
        .collect();
    let resource: Vec<Value> = resource.iter().map(|(k, v)| attr(k, v)).collect();
    serde_json::to_vec(&json!({
        "resourceMetrics": [{
            "resource": {"attributes": resource},
            "scopeMetrics": [{"metrics": [{
                "name": TOKEN_METRIC,
                "sum": {"aggregationTemporality": temporality, "isMonotonic": true, "dataPoints": points},
            }]}],
        }],
    }))
    .unwrap()
}

const ORCH: &[(&str, &str)] = &[("anthrex.run", "r1"), ("anthrex.role", "orchestrator")];

fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenUsage {
    TokenUsage {
        input,
        output,
        cache_read,
        cache_write,
    }
}

fn applied(ledger: &mut OtlpLedger, body: &[u8]) -> Vec<(String, String)> {
    ledger.apply(&parse_metrics(body).expect("the body parses"))
}

#[test]
fn parses_the_recorded_fixture() {
    let points = parse_metrics(FIXTURE).expect("the fixture parses");
    let point = |kind: UsageKind, value: u64| UsagePoint {
        run_id: "r-fix".into(),
        role: "orchestrator".into(),
        session_id: "00000000-0000-4000-8000-000000000001".into(),
        model: "claude-haiku-4-5".into(),
        kind,
        value,
        cumulative: false,
    };
    assert_eq!(
        points,
        vec![
            point(UsageKind::Input, 10),
            point(UsageKind::Output, 170),
            point(UsageKind::CacheRead, 13_689),
            point(UsageKind::CacheWrite, 12_503),
        ]
    );
    let mut ledger = OtlpLedger::default();
    assert_eq!(
        ledger.apply(&points),
        vec![("r-fix".to_string(), "orchestrator".to_string())]
    );
    assert_eq!(
        ledger.total("r-fix", "orchestrator"),
        usage(10, 170, 13_689, 12_503)
    );
}

#[test]
fn delta_points_add() {
    let mut ledger = OtlpLedger::default();
    let first = body(ORCH, 1, &[("input", json!(10)), ("output", json!(5))]);
    let second = body(ORCH, 1, &[("input", json!(3)), ("cacheCreation", json!(7))]);
    applied(&mut ledger, &first);
    let touched = applied(&mut ledger, &second);
    assert_eq!(
        touched,
        vec![("r1".to_string(), "orchestrator".to_string())]
    );
    assert_eq!(ledger.total("r1", "orchestrator"), usage(13, 5, 0, 7));
}

#[test]
fn cumulative_series_add_only_their_increase() {
    let mut ledger = OtlpLedger::default();
    applied(&mut ledger, &body(ORCH, 2, &[("input", json!(10))]));
    applied(&mut ledger, &body(ORCH, 2, &[("input", json!(25))]));
    applied(&mut ledger, &body(ORCH, 2, &[("input", json!(25))]));
    assert_eq!(ledger.total("r1", "orchestrator"), usage(25, 0, 0, 0));
    // `asInt`, as a number and as protobuf-JSON's string, reads the same.
    let mut as_int: Value = serde_json::from_slice(&body(ORCH, 2, &[("input", json!(0))])).unwrap();
    let point =
        &mut as_int["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0];
    point.as_object_mut().unwrap().remove("asDouble");
    point["asInt"] = json!("30");
    applied(&mut ledger, &serde_json::to_vec(&as_int).unwrap());
    assert_eq!(ledger.total("r1", "orchestrator"), usage(30, 0, 0, 0));
}

#[test]
fn a_cumulative_reset_restarts_the_series() {
    let mut ledger = OtlpLedger::default();
    applied(&mut ledger, &body(ORCH, 2, &[("output", json!(100))]));
    applied(&mut ledger, &body(ORCH, 2, &[("output", json!(4))]));
    assert_eq!(ledger.total("r1", "orchestrator"), usage(0, 104, 0, 0));
    applied(&mut ledger, &body(ORCH, 2, &[("output", json!(10))]));
    assert_eq!(ledger.total("r1", "orchestrator"), usage(0, 110, 0, 0));
}

#[test]
fn points_without_anthrex_run_are_ignored() {
    let no_run = body(
        &[("anthrex.role", "orchestrator")],
        1,
        &[("input", json!(10))],
    );
    assert_eq!(parse_metrics(&no_run), Ok(vec![]));
    let empty_run = body(&[("anthrex.run", "")], 1, &[("input", json!(10))]);
    assert_eq!(parse_metrics(&empty_run), Ok(vec![]));
    // Another role is kept in the ledger, under its own pair.
    let mut ledger = OtlpLedger::default();
    let worker = body(
        &[("anthrex.run", "r1"), ("anthrex.role", "worker")],
        1,
        &[("input", json!(2))],
    );
    applied(&mut ledger, &worker);
    assert_eq!(ledger.total("r1", "worker"), usage(2, 0, 0, 0));
    assert_eq!(ledger.total("r1", "orchestrator"), TokenUsage::default());
}

#[test]
fn other_metrics_are_ignored() {
    let mut value: Value = serde_json::from_slice(&body(ORCH, 1, &[("input", json!(10))])).unwrap();
    value["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
        json!("claude_code.cost.usage");
    assert_eq!(
        parse_metrics(&serde_json::to_vec(&value).unwrap()),
        Ok(vec![])
    );
    // A gauge of the token metric, an unknown `type`, a negative, fractional-overflowing
    // or non-numeric value, and an unspecified temporality are each dropped too.
    let odd = serde_json::to_vec(&json!({"resourceMetrics": [{
        "resource": {"attributes": [attr("anthrex.run", "r1")]},
        "scopeMetrics": [{"metrics": [
            {"name": TOKEN_METRIC, "gauge": {"dataPoints": [{"asDouble": 5}]}},
            {"name": TOKEN_METRIC, "sum": {"aggregationTemporality": 0, "dataPoints": [
                {"attributes": [attr("type", "input")], "asDouble": 5}]}},
            {"name": TOKEN_METRIC, "sum": {"aggregationTemporality": 1, "dataPoints": [
                {"attributes": [attr("type", "thinking")], "asDouble": 5},
                {"attributes": [attr("type", "input")], "asDouble": -5},
                {"attributes": [attr("type", "input")], "asDouble": "NaN"},
                {"attributes": [attr("type", "input")], "asInt": "12x"},
                {"attributes": [attr("type", "input")]},
            ]}},
        ]}],
    }]}))
    .unwrap();
    assert_eq!(parse_metrics(&odd), Ok(vec![]));
}

#[test]
fn malformed_json_is_an_error() {
    assert!(parse_metrics(b"{\"resourceMetrics\": [").is_err());
    assert!(parse_metrics(b"not json").is_err());
    assert!(parse_metrics(b"[]").is_err());
    assert!(parse_metrics(&[0xff, 0xfe]).is_err());
    // Deep nesting is refused by the parser's own recursion limit, never a stack overflow.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(parse_metrics(deep.as_bytes()).is_err());
}

#[test]
fn huge_values_saturate_and_the_ledger_is_bounded() {
    let mut ledger = OtlpLedger::default();
    applied(&mut ledger, &body(ORCH, 1, &[("input", json!(1e300))]));
    applied(&mut ledger, &body(ORCH, 1, &[("input", json!(1e300))]));
    assert_eq!(ledger.total("r1", "orchestrator").input, u64::MAX);
    for i in 0..(MAX_TOTALS + 10) {
        let run = format!("r{i}");
        applied(
            &mut ledger,
            &body(&[("anthrex.run", &run)], 1, &[("input", json!(1))]),
        );
    }
    assert_eq!(ledger.totals.len(), MAX_TOTALS);
    let long = "r".repeat(MAX_ID_BYTES + 1);
    assert_eq!(
        parse_metrics(&body(&[("anthrex.run", &long)], 1, &[("input", json!(1))])),
        Ok(vec![])
    );
}

#[test]
fn orchestrator_env_is_exact() {
    let env = orchestrator_env("http://127.0.0.1:4318", "r-7a2c");
    let expected: Vec<(String, String)> = [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
        ("OTEL_METRICS_EXPORTER", "otlp"),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318"),
        ("OTEL_METRIC_EXPORT_INTERVAL", "1000"),
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            "anthrex.run=r-7a2c,anthrex.role=orchestrator",
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(env, expected);
}

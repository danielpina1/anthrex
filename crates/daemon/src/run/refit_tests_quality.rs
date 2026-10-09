//! Task M9.5.8: threshold proposals, dismissing and applying. Milestone 9.8 decision 30
//! (task M9.8.13): the route proposals, the quality evidence that held a class's route
//! (ruling RH-2) and the model lists' effect on them are gone with the route refit.

use proto::{HistoryLine, SizeThresholds, TuningChange, TuningFile};

use super::tests::{NOW, fixture_records, record};
use super::{apply, dismiss, proposals, refit};

fn ids(lines: &[HistoryLine], file: &TuningFile, cfg: &config::Orchestrator) -> Vec<String> {
    proposals(lines, file, cfg)
        .into_iter()
        .map(|p| p.id)
        .collect()
}

#[test]
fn thresholds_m_is_dropped_when_not_above_s() {
    // 30 M tasks of 41..=70 lines: p90 is 67, rounded up to 70.
    let lines: Vec<HistoryLine> = (0..30u32)
        .map(|i| {
            let mut r = record(
                &format!("m{}", i / 10),
                i,
                100 + u64::from(i),
                60,
                1500,
                41 + i,
            );
            r.final_size = proto::Size::M;
            r.planned_size = proto::Size::M;
            HistoryLine::Task(r)
        })
        .collect();
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let p = proposals(&lines, &file, &cfg);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(p[0].id, "thresholds.m");
    assert_eq!(
        p[0].text,
        "M line threshold 100 → 70 (p90 of 30 merged M tasks)"
    );
    assert_eq!(
        p[0].change,
        TuningChange::Threshold {
            class: "m".into(),
            lines: 70
        }
    );
    // Not above an applied S threshold of 70: dropped.
    let mut applied = TuningFile {
        thresholds: Some(SizeThresholds {
            s_lines: 70,
            m_lines: 100,
        }),
        ..TuningFile::default()
    };
    assert!(ids(&lines, &applied, &cfg).is_empty());
    applied.thresholds = Some(SizeThresholds {
        s_lines: 65,
        m_lines: 100,
    });
    assert_eq!(ids(&lines, &applied, &cfg), ["thresholds.m"]);
}

#[test]
fn dismissed_is_not_proposed_until_the_value_changes() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let current = proposals(&lines, &file, &cfg);
    let dismissed = dismiss(&file, &current, &["thresholds.s".to_string()]).unwrap();
    assert_eq!(
        dismissed.dismissed.get("thresholds.s").map(String::as_str),
        Some("35")
    );
    assert!(ids(&lines, &dismissed, &cfg).is_empty());
    // History moves the proposal to 40: it is proposed again.
    let mut more = lines.clone();
    for line in &mut more {
        if let HistoryLine::Task(r) = line
            && let Some(d) = r.diff.as_mut()
        {
            d.added += 5;
        }
    }
    assert_eq!(ids(&more, &dismissed, &cfg), ["thresholds.s"]);
    assert_eq!(proposals(&more, &dismissed, &cfg)[0].proposed, "40");
    // A dismissed value is not current, so it cannot be applied.
    let current = proposals(&lines, &dismissed, &cfg);
    assert!(apply(&dismissed, &current, &["thresholds.s".to_string()]).is_err());
}

#[test]
fn apply_writes_only_the_named_change() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let current = proposals(&lines, &file, &cfg);
    let applied = apply(&file, &current, &["thresholds.s".to_string()]).unwrap();
    let mut expected = file.clone();
    expected.thresholds = Some(SizeThresholds {
        s_lines: 35,
        m_lines: 100,
    });
    assert_eq!(applied, expected);
    // Once applied, it is not proposed again.
    assert!(proposals(&lines, &applied, &cfg).is_empty());
}

#[test]
fn an_unknown_id_is_refused_whole() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let current = proposals(&lines, &file, &cfg);
    let named = ["thresholds.s".to_string(), "route.m".to_string()];
    let refusal = "no current proposal route.m; run anthrex run stats to see the proposals";
    assert_eq!(apply(&file, &current, &named), Err(refusal.to_string()));
    assert_eq!(dismiss(&file, &current, &named), Err(refusal.to_string()));
}

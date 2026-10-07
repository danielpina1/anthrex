//! Decision 17's JSON schemas, exactly as the brief's Interfaces give them. Every
//! object is closed and lists every property in `required`; optional values are
//! nullable. `parse` enforces the same limits by hand. Pure.

use super::DeciderKind;
use serde_json::Value;

/// The schemas, keyed by the kind's label (M8b's "Decider schemas (exact)", 9.2's
/// `ci_summary` from its Interfaces, and the run title change's `run_name`).
const SCHEMAS: &str = r#"{"triage":{"type":"object","additionalProperties":false,"required":["kinds","scale","reason","task"],"properties":{
  "kinds":{"type":"array","minItems":1,"maxItems":4,"items":{"enum":["code","docs","research","review"]}},
  "scale":{"enum":["single","plan","large"]},
  "reason":{"type":"string","minLength":1,"maxLength":500},
  "task":{"anyOf":[{"type":"null"},{"type":"object","additionalProperties":false,
    "required":["title","brief","acceptance","owns","size","interface_change","test_mode","test_mode_reason","test_to_write"],
    "properties":{
      "title":{"type":"string","minLength":1,"maxLength":120},
      "brief":{"type":"string","minLength":1,"maxLength":4000},
      "acceptance":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","minLength":1,"maxLength":500}},
      "owns":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","minLength":1,"maxLength":300}},
      "size":{"enum":["S","M"]},
      "interface_change":{"type":"boolean"},
      "test_mode":{"enum":["tdd","check","none"]},
      "test_mode_reason":{"type":["string","null"],"maxLength":300},
      "test_to_write":{"type":["string","null"],"maxLength":300}}}]}}},
 "size_check":{"type":"object","additionalProperties":false,"required":["tasks"],"properties":{
  "tasks":{"type":"array","maxItems":50,"items":{"type":"object","additionalProperties":false,"required":["id","size","reason"],
    "properties":{"id":{"type":"string","minLength":1,"maxLength":16},"size":{"enum":["S","M","L"]},
                  "reason":{"type":"string","minLength":1,"maxLength":500}}}}}},
 "check_summary":{"type":"object","additionalProperties":false,"required":["lines"],"properties":{
  "lines":{"type":"array","minItems":1,"maxItems":40,"items":{"type":"string","maxLength":300}}}},
 "blocked_reason":{"type":"object","additionalProperties":false,"required":["kind","reason"],"properties":{
  "kind":{"enum":["question","mis_sized","environment"]},"reason":{"type":"string","minLength":1,"maxLength":300}}},
 "ci_summary":{"type":"object","additionalProperties":false,"required":["lines","failing_tests","category"],"properties":{
  "lines":{"type":"array","minItems":1,"maxItems":40,"items":{"type":"string","maxLength":300}},
  "failing_tests":{"type":"array","maxItems":50,"items":{"type":"string","minLength":1,"maxLength":200}},
  "category":{"enum":["test","build","lint","infra","unknown"]}}},
 "run_name":{"type":"object","additionalProperties":false,"required":["title","slug"],"properties":{
  "title":{"type":"string","minLength":1,"maxLength":60},
  "slug":{"type":"string","minLength":1,"maxLength":32,"pattern":"^[a-z0-9]+(-[a-z0-9]+)*$"}}}}"#;

/// The schema of `kind`'s answer.
pub fn schema(kind: DeciderKind) -> Value {
    let mut all: Value = serde_json::from_str(SCHEMAS).expect("the decider schemas are JSON");
    all[kind.label()].take()
}

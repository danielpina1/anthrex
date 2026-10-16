//! The controller's ruling (task M9.15.9; task 8's review, I3): a known failure's title,
//! body and headlines come from stored signature headlines, which are redacted, so a
//! failure that printed a token never brings it to a prompt or the Lessons page.

use super::*;
use crate::lessons::signature::of;

#[test]
fn a_known_failure_from_a_token_bearing_failure_is_redacted() {
    let token = format!("ghp_{}", "FAKE".repeat(9));
    let sig = of(&format!("error: auth failed for {token}\n")).expect("a signature");
    let history = [1, 2].map(|n| {
        line(failed(
            task(&rid(n), "t1", T0 + u64::from(n) * DAY, TaskOutcome::Blocked),
            &sig.fp,
            &sig.headline,
        ))
    });
    let got = known(&history, &Store::default(), T0 + 3 * DAY, &cfg());
    assert_eq!(ids(&got), vec![kf(&sig.fp)]);
    let k = &got[0];
    let text = format!("{} {} {:?}", k.title, k.body, k.headlines);
    for fragment in ["ghp_", "FAKEFAKE"] {
        assert!(!text.contains(fragment), "{text}");
    }
    assert_eq!(
        k.title,
        "Known failure: error: auth failed for [redacted token]"
    );
}

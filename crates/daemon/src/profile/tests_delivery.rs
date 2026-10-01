//! Milestone 9.2 task M9.2.3: the profile's `[delivery]` table through `anthrex profile
//! edit` (decision 3), and that it never reaches a run's profile hash.

use std::path::Path;

use proto::{DeliveryMode, DeliveryProfile, ProfileSpec, RepoProfile};

use super::proposal::{EDIT_KEYS, apply_edit, validate};
use super::resolve::run_profile;

fn stored() -> RepoProfile {
    RepoProfile {
        check: Some("cargo test".into()),
        single_test: Some("cargo test -- --exact {test}".into()),
        test_passed: Some("test {test} ... ok".into()),
        sample_test: Some("a::b".into()),
        ..Default::default()
    }
}

fn pr(remote: &str) -> Option<DeliveryProfile> {
    Some(DeliveryProfile {
        mode: DeliveryMode::Pr,
        remote: remote.into(),
    })
}

#[test]
fn profile_edit_sets_delivery_mode_and_remote() {
    let base = stored();
    assert_eq!(base.delivery, None);

    // `delivery.mode` makes the table, with the default remote.
    let (edited, reverify) = apply_edit(&base, "delivery.mode", Some("pr")).unwrap();
    assert_eq!(edited.delivery, pr("origin"));
    assert!(
        !reverify,
        "delivery changes no command, so nothing re-verifies"
    );
    let rest = RepoProfile {
        delivery: None,
        ..edited.clone()
    };
    assert_eq!(rest, base, "nothing else changed");

    // `delivery.remote` keeps the mode.
    let (edited, reverify) = apply_edit(&edited, "delivery.remote", Some("upstream")).unwrap();
    assert_eq!(edited.delivery, pr("upstream"));
    assert!(!reverify);
    let (local, _) = apply_edit(&edited, "delivery.mode", Some("local")).unwrap();
    assert_eq!(
        local.delivery,
        Some(DeliveryProfile {
            mode: DeliveryMode::Local,
            remote: "upstream".into(),
        })
    );

    // A remote set first makes a `local` table: absent means local.
    let (remote_only, _) = apply_edit(&base, "delivery.remote", Some("fork")).unwrap();
    assert_eq!(
        remote_only.delivery,
        Some(DeliveryProfile {
            mode: DeliveryMode::Local,
            remote: "fork".into(),
        })
    );

    // `--unset`: the mode drops the whole table, the remote goes back to `origin`.
    let (unset, _) = apply_edit(&edited, "delivery.remote", None).unwrap();
    assert_eq!(unset.delivery, pr("origin"));
    let (unset, _) = apply_edit(&edited, "delivery.mode", None).unwrap();
    assert_eq!(unset.delivery, None);
    assert_eq!(unset, base);
    let (unset, _) = apply_edit(&base, "delivery.remote", None).unwrap();
    assert_eq!(
        unset.delivery, None,
        "unsetting on an absent table is a no-op"
    );

    // The stored file round-trips with the table.
    let text = toml::to_string(&edited).unwrap();
    assert!(
        text.contains("[delivery]\nmode = \"pr\"\nremote = \"upstream\"\n"),
        "{text}"
    );
    assert_eq!(toml::from_str::<RepoProfile>(&text).unwrap(), edited);
}

#[test]
fn profile_edit_refuses_an_unknown_delivery_key() {
    let base = stored();
    let delivery_keys = "delivery.mode, delivery.remote";
    for key in [
        "delivery.sync",
        "delivery.poll_secs",
        "delivery.",
        "delivery.mode.x",
    ] {
        assert_eq!(
            apply_edit(&base, key, Some("x")),
            Err(format!("unknown key {key}; one of {delivery_keys}")),
            "{key}"
        );
    }
    // The bare table is not a key; the generic list names the two.
    let keys = EDIT_KEYS.join(", ");
    assert!(keys.contains("delivery.mode, delivery.remote"), "{keys}");
    assert_eq!(
        apply_edit(&base, "delivery", Some("pr")),
        Err(format!("unknown key delivery; one of {keys}"))
    );
    // A mode is `pr` or `local`.
    for bad in ["PR", "github", "", "true"] {
        assert_eq!(
            apply_edit(&base, "delivery.mode", Some(bad)),
            Err("delivery.mode: must be pr or local".to_string()),
            "{bad:?}"
        );
    }
    // A remote must be a valid remote name: `git check-ref-format --branch`'s rule,
    // checked against git 2.50.1 (M9.2.3's review fix 4).
    for bad in [
        "",
        "a.",
        "x/a.",
        "HEAD",
        "has space",
        "-x",
        ".x",
        "a..b",
        "a:b",
        "a~b",
        "a^b",
        "a?b",
        "a*b",
        "a[b",
        "a\\b",
        "x.lock",
        "x/",
        "a//b",
        "a@{b",
    ] {
        assert_eq!(
            apply_edit(&base, "delivery.remote", Some(bad)),
            Err(format!("delivery.remote: {bad} is not a valid remote name")),
            "{bad:?}"
        );
    }
    for ok in [
        "origin",
        "up-stream",
        "team/fork",
        "my_remote2",
        "@",
        "a/HEAD",
        "HEAD/x",
        "a.b",
        "a@b",
    ] {
        assert!(
            apply_edit(&base, "delivery.remote", Some(ok)).is_ok(),
            "{ok}"
        );
    }
    // M8b's validation reports a stored profile's bad remote, too.
    let bad = RepoProfile {
        delivery: pr("a..b"),
        ..base
    };
    assert_eq!(
        validate(&bad),
        vec!["delivery.remote: a..b is not a valid remote name".to_string()]
    );
}

/// M9.2.3's review fix 5: a refused remote is not echoed verbatim. A URL's userinfo
/// (a token, say) is redacted, and control characters are escaped.
#[test]
fn a_refused_remote_is_redacted_in_the_error() {
    let base = stored();
    let refused = |name: &str| apply_edit(&base, "delivery.remote", Some(name)).unwrap_err();
    let url = refused("https://u:tok@github.com/o/r");
    assert!(!url.contains("tok"), "{url}");
    assert_eq!(
        url,
        "delivery.remote: https://***@github.com/o/r is not a valid remote name"
    );
    let scp = refused("u:tok@github.com:o/r");
    assert!(!scp.contains("tok"), "{scp}");
    assert_eq!(
        scp,
        "delivery.remote: ***@github.com:o/r is not a valid remote name"
    );
    let control = refused("a\tb\x1b[2Jc");
    assert!(!control.chars().any(|c| c.is_control()), "{control:?}");
    assert_eq!(
        control,
        "delivery.remote: a\\tb\\u{1b}[2Jc is not a valid remote name"
    );
    // A stored profile's bad remote is reported the same way.
    let bad = RepoProfile {
        delivery: pr("https://u:tok@github.com/o/r"),
        ..base.clone()
    };
    assert_eq!(
        validate(&bad),
        vec!["delivery.remote: https://***@github.com/o/r is not a valid remote name".to_string()]
    );
}

/// **Pinning** (decision 3, last bullet): two stored profiles that differ only in their
/// `[delivery]` table give a run the same profile, so the same `Run.profile_hash`:
/// `delivery` is frozen into `RunDelivery`, never into the hashed `Profile`.
#[test]
fn delivery_does_not_change_the_profile_hash() {
    let local = stored();
    let pr_mode = RepoProfile {
        delivery: pr("upstream"),
        ..stored()
    };
    let none = ProfileSpec::default();
    let path = Path::new("/data/repos/r-00000000/profile.toml");
    let a = run_profile(Some(&local), path, &none, &none);
    let b = run_profile(Some(&pr_mode), path, &none, &none);
    assert_eq!(a, b, "the chosen profile ignores delivery");
    let hash = |spec: &ProfileSpec| {
        crate::run::tiers::profile_hash(&crate::run::plan::resolve_profile(spec, &none))
    };
    assert_eq!(hash(&a.spec), hash(&b.spec));
    assert_eq!(local.spec(), pr_mode.spec());
}

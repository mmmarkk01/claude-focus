//! Locks in the Phase 3.3 "single source of version truth" invariant. The two
//! version schemes are intentionally INCOMPATIBLE — the crate uses semver
//! (Cargo.toml) while GNOME mandates a bare integer in extension/metadata.json
//! — so this asserts each scheme is well-formed rather than that they are equal.
//! See the "Versioning" section of README.md for the documented bump rule.

const METADATA_JSON: &str = include_str!("../extension/metadata.json");

#[test]
fn metadata_version_is_a_positive_integer() {
    let v: serde_json::Value =
        serde_json::from_str(METADATA_JSON).expect("metadata.json is valid JSON");
    let version = v.get("version").expect("metadata.json has a version field");
    let n = version
        .as_u64()
        .expect("GNOME requires metadata.json version to be an integer");
    assert!(n >= 1, "the extension revision starts at 1");
}

#[test]
fn cargo_version_is_semver() {
    // CARGO_PKG_VERSION is the crate's semver; `claude-focus --version` prints
    // it (Phase 1.5). Assert MAJOR.MINOR.PATCH so the scheme can't silently
    // drift into something GNOME's integer rule would be confused with.
    let v = env!("CARGO_PKG_VERSION");
    let parts: Vec<&str> = v.split('.').collect();
    assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {v:?}");
    for p in parts {
        assert!(
            !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()),
            "non-numeric semver component in {v:?}"
        );
    }
}

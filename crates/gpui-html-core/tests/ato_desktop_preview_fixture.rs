//! Acceptance fixture regression test for issue #9.
//!
//! Closes the loop on the v0.1 design goal: the original Ato Desktop
//! preview HTML — DOCTYPE + `<html>` + `<head>` with meta/link/script/
//! style + `<body>` + UI tree — compiles end-to-end with **no manual
//! edits** and produces a stable gpui builder Rust output.
//!
//! The fixture (`examples/ato-desktop-preview.gpui.html`) is intentionally
//! the un-modified mock that was the project's starting input. Rules
//! the rest of this file enforces:
//!
//! - The fixture stays full HTML. No stripping of `<!DOCTYPE>` /
//!   `<html>` / `<head>` / `<body>`. No substituting `text-accent-foreground`
//!   for `text-accent_foreground`, no replacing `max-w-128` with
//!   `max-w-12`, no dropping `font-sans` from the markup.
//! - The compiled output is pinned by a sibling snapshot file
//!   (`examples/ato-desktop-preview.expected.gpui.txt`). Any change to
//!   the lowering surface that affects this fixture must update both
//!   the fixture (if input changed) and the snapshot in the same PR.
//! - Structural counts and key substrings catch the most common
//!   regression shapes (e.g. a class accidentally going UnknownClass,
//!   a no-op accidentally re-emitting, a theme-token normalization
//!   getting reverted).
//!
//! If this test breaks, read SKILL.md before "fixing" it — the
//! fixture's rules are documented there.

const FIXTURE_HTML: &str = include_str!("../../../examples/ato-desktop-preview.gpui.html");
const FIXTURE_EXPECTED: &str =
    include_str!("../../../examples/ato-desktop-preview.expected.gpui.txt");

/// Byte count of the fixture as it ships. Acts as a canary against
/// accidental edits — any modification (whitespace, character
/// substitution, line ending change) will trip this before the
/// compile-output assertions further down, making the failure mode
/// obvious.
///
/// If the fixture is intentionally modified, update this number and
/// the SHA256 in `examples/ato-desktop-preview.gpui.html.sha256` in
/// the same PR, and confirm the change is documented as
/// "fixture intentionally updated" rather than "drift".
const FIXTURE_BYTES: usize = 12608;

/// SHA256 of the fixture as it ships. The companion `.sha256` file
/// next to the fixture documents the same hash; both must agree.
const FIXTURE_SHA256: &str = "8ea02d35ad441794a049bb76792e69772dcaea8e4a5c42cfea3fde8e8a401405";

#[test]
fn fixture_byte_count_pins_no_manual_edits() {
    assert_eq!(
        FIXTURE_HTML.len(),
        FIXTURE_BYTES,
        "fixture has been modified — update FIXTURE_BYTES if the change \
         is intentional, but also update the .sha256 file and verify the \
         change is what you actually meant."
    );
}

#[test]
fn fixture_has_no_manual_substitutions() {
    // Quick sanity check: the fixture should contain the exact class
    // names that #7 / #19 enabled. If a contributor "fixes" a
    // perceived bug by reverting the class names in the fixture, this
    // test catches it.
    let must_contain_verbatim = [
        // #7 — hyphenated theme tokens (no underscore substitution)
        "text-accent-foreground",
        // #19 — app-shell utilities (no v0.2-manifest workaround,
        // no per-element font-family removal, no viewport-keyword
        // substitution)
        "max-w-128",
        "font-sans",
        "h-screen",
        "w-screen",
        // #26 — HTML5 boilerplate is not stripped
        "<!DOCTYPE html>",
        "<html lang=\"ja\">",
        "<head>",
        "<body>",
        "<script>",
        "<style>",
        "</html>",
    ];
    for needle in must_contain_verbatim {
        assert!(
            FIXTURE_HTML.contains(needle),
            "fixture must contain `{needle}` verbatim (no manual edits) — \
             see SKILL.md § acceptance fixture"
        );
    }
}

#[test]
fn fixture_compiles_end_to_end_with_no_manual_edits() {
    // The core #9 contract: compile must succeed on the original full
    // HTML, no stripping.
    let out = gpui_html_core::compile(FIXTURE_HTML)
        .expect("Ato Desktop preview fixture must compile without error");

    // Key contract substrings — these pin the deliverables for the
    // design-decision PRs.
    let must_appear = [
        // #19 viewport keywords
        ".h_screen()",
        ".w_screen()",
        // #19 max-w-128 app-shell compat exemption
        ".max_w(rems(32.0))",
        // #7 hyphen-to-snake normalization
        ".text_color(theme.accent_foreground)",
        // #15 bg-transparent literal (not theme.transparent)
        "gpui::transparent_black()",
    ];
    for needle in must_appear {
        assert!(
            out.contains(needle),
            "compiled fixture output is missing `{needle}` — \
             see SKILL.md for the design rationale behind this lowering"
        );
    }

    // #19 font-sans no-op: must NOT emit a `.font_sans()` or similar.
    assert!(
        !out.contains("font_sans"),
        "compiled fixture output must not include `font_sans` — \
         `font-sans` is a recognized no-op per issue #19, font-family \
         is the host app/Theme's concern"
    );
}

#[test]
fn fixture_structural_counts_match_expected_subtree() {
    // Element / child / id counts catch shape-level regressions even
    // when the substring asserts above happen to pass. Numbers derived
    // from the fixture as it ships; update in the same PR as any
    // intentional fixture or lowering change.
    let out = gpui_html_core::compile(FIXTURE_HTML).unwrap();
    assert_eq!(count(&out, "div()"), 69, "div() element count");
    assert_eq!(count(&out, "span()"), 41, "span() element count");
    assert_eq!(count(&out, ".id("), 17, ".id(\"...\") preserved count");
    assert_eq!(count(&out, ".child("), 170, ".child(...) boundary count");
}

#[test]
fn fixture_full_output_snapshot() {
    // The exhaustive regression guard: byte-equal the entire compiled
    // output against the sibling snapshot file. Any lowering change
    // that affects this fixture must update
    // `examples/ato-desktop-preview.expected.gpui.txt` in the same PR.
    //
    // `trim_end_matches('\n')` lets the snapshot file end with the
    // usual editor-added trailing newline without the comparison
    // failing — `compile()` doesn't emit one.
    let out = gpui_html_core::compile(FIXTURE_HTML).unwrap();
    let expected = FIXTURE_EXPECTED.trim_end_matches('\n');
    assert_eq!(
        out, expected,
        "compiled output diverged from \
         examples/ato-desktop-preview.expected.gpui.txt"
    );
}

#[test]
fn fixture_sha256_constant_is_64_hex_chars() {
    // Cheap sanity check on the SHA256 constant shape — the actual
    // hash is verified by the `.sha256` companion file outside the
    // test suite. This guards against typos.
    assert_eq!(FIXTURE_SHA256.len(), 64);
    assert!(FIXTURE_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

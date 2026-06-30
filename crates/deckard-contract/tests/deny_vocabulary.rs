//! Freezes the Deny-reason vocabulary (issue #28). Three mutually-reinforcing guards, all
//! running under plain `cargo test --workspace` with no extra dependencies:
//!
//!   1. [`every_deny_reason_routes_through_the_frozen_vocabulary`] — a structure-aware scan of
//!      the production sources of `deckard-contract`, `deckard-signerd`, and `deckard-mcp`. For
//!      every `Decision::Deny` / `ExecuteResult::Denied` / `SignOrderResult::Denied` /
//!      `ApprovalStatus::Denied` construction and every `reply_error(..)` call, the `reason`
//!      value MUST be a [`deny_reasons`](deckard_contract::deny_reasons) const / builder, or a
//!      passthrough of an already-frozen stored reason. A raw literal, a foreign variable, a
//!      foreign const, or a free-form prefix all fail. It is precise: `ReadStatus`/
//!      `ShieldStatus` reasons (a different, free-form vocabulary) are never inspected.
//!
//!   2. [`frozen_set_matches_module_exports`] — parses `deny_reasons.rs` and asserts its
//!      `pub const … : &str` set is exactly the hand-listed [`FROZEN`] snapshot, and that the
//!      module exports exactly the four dynamic-prefix builders — so neither a new const nor a
//!      new `pub fn` can be added to the trusted module and slip past the guards.
//!
//!   3. [`frozen_set_is_exactly_documented`] — pins the snapshot's size, uniqueness, shape,
//!      and the dynamic-prefix separator. Bumping it is the deliberate gate that should also
//!      add the tag's row to `docs/build/31-agent-quickstart.md`.
//!
//! ## How the scan stays honest
//!
//! The source is first *masked*: the interior of every string literal and every line comment
//! is blanked to spaces (length and `\n` preserved). Brace/paren/comma counting then can't be
//! fooled by `{`, `,`, or `//` that live inside a string or a comment, and a raw literal at a
//! Deny site still shows its `"` delimiters (so it's caught). Then every `#[cfg(test)]` block
//! is blanked in place, so in-file test modules (which legitimately assert against literals)
//! are skipped while production code *after* a test module is still scanned.

use std::fs;
use std::path::{Path, PathBuf};

use deckard_contract::deny_reasons as r;

/// `(crate dir relative to this crate's manifest, subdir to scan)`.
const SCAN: &[(&str, &str)] = &[
    (".", "src"), // deckard-contract (this crate)
    ("../deckard-signerd", "src"),
    ("../deckard-mcp", "src"),
];

/// The struct-literal heads that mint a Deny reason. Their match-PATTERN forms use shorthand
/// (`{ reason }`, `{ .. }`) with no `reason:` field and are skipped automatically.
const DENY_MARKERS: &[&str] = &[
    "Decision::Deny",
    "ExecuteResult::Denied",
    "SignOrderResult::Denied",
    "ApprovalStatus::Denied",
];

/// The four dynamic-prefix builders `deny_reasons` is allowed to export (besides the consts).
const PREFIX_BUILDERS: &[&str] = &[
    "railgun_keys",
    "signer_error",
    "sign_failed",
    "broadcast_failed",
];

/// The complete frozen vocabulary: 38 static tags + 4 dynamic-prefix tags. Editing this list
/// is the deliberate gate — change it here, in `deny_reasons.rs`, AND (for a real, non-test
/// tag) in `docs/build/31-agent-quickstart.md`. `swap_unsupported_in_mock` is test-surface
/// only and is intentionally absent from the docs table.
const FROZEN: &[&str] = &[
    // policy gate
    r::REVOKED,
    r::OFF_ALLOWLIST,
    r::UNDECODABLE,
    r::OVER_CAP,
    r::NO_RULE,
    r::RECEIVER_ZERO,
    r::RECEIVER_NOT_WALLET,
    r::ZERO_AMOUNT,
    r::OFF_SWAP_LIST,
    r::VALID_TO_TOO_FAR,
    r::CHAINID_MISMATCH,
    r::ETH_SIGN_REFUSED,
    r::DELEGATION_REFUSED,
    // daemon process-level
    r::LOCKED,
    r::CHAIN_MISMATCH,
    r::UNSUPPORTED_V1,
    r::ERC20_UNSUPPORTED_V1,
    r::SHIELD_TO_MISMATCH,
    r::CAP_EXCEEDED,
    r::NOT_APPROVED,
    r::USER_DENIED,
    r::EXPIRED,
    r::UNKNOWN_REQUEST,
    r::ALREADY_EXECUTED,
    r::BROADCAST_TIMEOUT,
    r::MALFORMED_REQUEST,
    r::DERIVATION_UNVERIFIED,
    r::SHIELD_UNAVAILABLE,
    r::RESOLVE_NOT_AUTHORIZED,
    r::RESERVE_FAILED,
    // swap v1
    r::APPROVE_WITH_VALUE,
    r::APPROVE_WRONG_SPENDER,
    r::APPROVE_NO_MATCHING_ORDER,
    r::ALREADY_SIGNED,
    r::NOT_AN_ORDER,
    r::NOT_A_MESSAGE,
    r::NOT_A_TRANSACTION,
    r::SWAP_UNSUPPORTED_IN_MOCK,
    // dynamic prefixes
    r::RAILGUN_KEYS,
    r::SIGNER_ERROR,
    r::SIGN_FAILED,
    r::BROADCAST_FAILED,
];

// ───────────────────────── masking ─────────────────────────

/// Blank the interior of every `"…"` string literal and every `// …` line comment to spaces,
/// preserving byte length and newlines. (The scanned crates contain no raw strings, block
/// comments, or `'"'`/`'{'`-style char literals — verified — so a normal-string + line-comment
/// masker is exact here.)
fn mask(src: &str) -> String {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let mut i = 0;
    let mut in_str = false;
    let mut in_line = false;
    while i < n {
        let c = b[i];
        if in_line {
            if c == b'\n' {
                in_line = false;
            } else {
                out[i] = b' ';
            }
            i += 1;
        } else if in_str {
            if c == b'\\' {
                out[i] = b' ';
                if i + 1 < n && b[i + 1] != b'\n' {
                    out[i + 1] = b' ';
                }
                i += 2;
            } else if c == b'"' {
                in_str = false; // keep the closing quote
                i += 1;
            } else {
                if c != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
        } else if c == b'"' {
            in_str = true; // keep the opening quote
            i += 1;
        } else if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            in_line = true;
            out[i] = b' ';
            i += 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("masking preserves UTF-8 (only ASCII bytes blanked)")
}

/// Blank every `#[cfg(test)]`-attributed block in place (spaces, newlines kept) so in-file
/// test modules are skipped while production code before AND after them is still scanned.
/// Runs on already-masked text, so the brace balance can't be thrown off by string braces.
fn blank_cfg_test(masked: &str) -> String {
    let mut bytes = masked.as_bytes().to_vec();
    let mut from = 0usize;
    while let Some(rel) = masked[from..].find("#[cfg(test)]") {
        let attr = from + rel;
        let Some(brace_rel) = masked[attr..].find('{') else {
            blank_span(&mut bytes, attr, masked.len());
            break;
        };
        let brace = attr + brace_rel;
        let body = balanced(masked, brace, b'{', b'}');
        let end = (brace + 1 + body.len() + 1).min(masked.len());
        blank_span(&mut bytes, attr, end);
        from = end;
    }
    String::from_utf8(bytes).expect("blanking preserves UTF-8")
}

fn blank_span(bytes: &mut [u8], start: usize, end: usize) {
    let end = end.min(bytes.len());
    for b in &mut bytes[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

// ───────────────────────── structure helpers ─────────────────────────

/// Given the byte index of an opening delimiter, return the slice strictly inside the matching
/// close (balanced). "" if unbalanced.
fn balanced(text: &str, start: usize, open: u8, close: u8) -> &str {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return &text[start + 1..i];
            }
        }
        i += 1;
    }
    ""
}

/// Extract the `reason:` field value from a struct-literal body. `None` for a shorthand/pattern
/// body (`reason`, `..`) with no `reason:` field.
fn reason_value(body: &str) -> Option<&str> {
    let key = body.find("reason:")?;
    let after = &body[key + "reason:".len()..];
    let bytes = after.as_bytes();
    let (mut paren, mut brack, mut brace) = (0i32, 0i32, 0i32);
    let mut end = after.len();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'[' => brack += 1,
            b']' => brack -= 1,
            b'{' => brace += 1,
            b'}' => brace -= 1,
            b',' if paren == 0 && brack == 0 && brace == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    Some(after[..end].trim())
}

/// Split a comma-separated argument list at top-level commas (paren/brace/bracket aware).
fn split_top_level(args: &str) -> Vec<&str> {
    let bytes = args.as_bytes();
    let (mut paren, mut brack, mut brace) = (0i32, 0i32, 0i32);
    let mut parts = Vec::new();
    let mut start = 0usize;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'[' => brack += 1,
            b']' => brack -= 1,
            b'{' => brace += 1,
            b'}' => brace -= 1,
            b',' if paren == 0 && brack == 0 && brace == 0 => {
                parts.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    parts
}

/// Is `value` an allowed reason expression at a Deny construction site?
fn struct_reason_ok(value: &str) -> bool {
    // A frozen const or a typed prefix builder, e.g. `deny_reasons::REVOKED.into()` or
    // `deny_reasons::signer_error(one_line(&e))`. A raw literal would still carry its `"`
    // delimiters even after masking, so it is rejected here.
    if value.starts_with("deny_reasons::") && !value.contains('"') {
        return true;
    }
    // Re-raising an already-frozen, stored reason: `reason`, `reason.clone()`, `reason.into()`.
    value == "reason" || value.starts_with("reason.")
}

/// Is the 2nd argument to `reply_error(stream, <arg>)` allowed?
fn reply_error_arg_ok(arg: &str) -> bool {
    if arg.contains(": &str") {
        return true; // the `reply_error` definition itself, not a call site
    }
    if arg.starts_with("deny_reasons::") && !arg.contains('"') {
        return true;
    }
    // Exact passthroughs only — `&reason_code` (a foreign string) must NOT slip through.
    arg == "reason" || arg == "&reason" || arg.starts_with("reason.") || arg.starts_with("&reason.")
}

/// Char immediately after a marker must not continue an identifier.
fn boundary_ok(text: &str, end: usize) -> bool {
    text.as_bytes()
        .get(end)
        .is_none_or(|&b| !(b.is_ascii_alphanumeric() || b == b'_'))
}

fn line_of(text: &str, pos: usize) -> usize {
    text[..pos].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Scan one source string. `label` is used only in violation messages.
fn scan_source(label: &str, src: &str, violations: &mut Vec<String>) {
    let text = blank_cfg_test(&mask(src));

    for marker in DENY_MARKERS {
        for (idx, _) in text.match_indices(marker) {
            let end = idx + marker.len();
            if !boundary_ok(&text, end) {
                continue;
            }
            let rest = &text[end..];
            let trimmed = rest.trim_start();
            if !trimmed.starts_with('{') {
                continue;
            }
            let brace_at = end + (rest.len() - trimmed.len());
            let body = balanced(&text, brace_at, b'{', b'}');
            if let Some(value) = reason_value(body) {
                if !struct_reason_ok(value) {
                    violations.push(format!(
                        "{label}:{}  {marker} {{ reason: {value} }}",
                        line_of(&text, idx)
                    ));
                }
            }
        }
    }

    for (idx, _) in text.match_indices("reply_error(") {
        let paren_at = idx + "reply_error".len();
        let args = balanced(&text, paren_at, b'(', b')');
        if let Some(arg) = split_top_level(args).get(1) {
            if !reply_error_arg_ok(arg) {
                violations.push(format!(
                    "{label}:{}  reply_error(.., {arg})",
                    line_of(&text, idx)
                ));
            }
        }
    }
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// ───────────────────────────── tests ─────────────────────────────

#[test]
fn every_deny_reason_routes_through_the_frozen_vocabulary() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for (rel, sub) in SCAN {
        let dir = base.join(rel).join(sub);
        assert!(
            dir.is_dir(),
            "scan target missing (workspace layout changed?): {}",
            dir.display()
        );
        let mut files = Vec::new();
        rs_files(&dir, &mut files);
        for f in &files {
            if let Ok(src) = fs::read_to_string(f) {
                scan_source(&f.display().to_string(), &src, &mut violations);
            }
        }
    }
    assert!(
        violations.is_empty(),
        "every production Deny/Denied/reply_error reason must be a deckard_contract::deny_reasons \
         const or builder — never a raw literal, foreign variable, or free-form prefix. Add new \
         tags to that module + FROZEN + the docs table. Offenders:\n{}",
        violations.join("\n")
    );
}

#[test]
fn frozen_set_matches_module_exports() {
    let module =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/deny_reasons.rs"))
            .expect("read deny_reasons.rs");

    let mut consts: Vec<String> = Vec::new();
    let mut pub_fns: Vec<String> = Vec::new();
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("pub const ") {
            // rest = `NAME: &str = "VALUE";`
            if let Some(q1) = rest.find('"') {
                if let Some(rel) = rest[q1 + 1..].find('"') {
                    consts.push(rest[q1 + 1..q1 + 1 + rel].to_string());
                }
            }
        } else if let Some(rest) = t.strip_prefix("pub fn ") {
            if let Some(paren) = rest.find('(') {
                pub_fns.push(rest[..paren].trim().to_string());
            }
        }
    }

    let mut consts_sorted = consts.clone();
    consts_sorted.sort();
    let mut frozen_sorted: Vec<String> = FROZEN.iter().map(|s| (*s).to_string()).collect();
    frozen_sorted.sort();
    assert_eq!(
        consts_sorted, frozen_sorted,
        "deny_reasons.rs `pub const` tags and the FROZEN snapshot diverged — update both \
         (and the docs table for a real tag)"
    );

    let mut fns_sorted = pub_fns.clone();
    fns_sorted.sort();
    let mut expected_fns: Vec<String> = PREFIX_BUILDERS.iter().map(|s| (*s).to_string()).collect();
    expected_fns.sort();
    assert_eq!(
        fns_sorted, expected_fns,
        "deny_reasons.rs must export exactly the four dynamic-prefix builders — a new `pub fn` \
         would be a reason source the scan trusts blindly; gate it deliberately"
    );
}

#[test]
fn frozen_set_is_exactly_documented() {
    assert_eq!(
        FROZEN.len(),
        42,
        "added/removed a Deny tag? update FROZEN, deny_reasons.rs, and the docs table"
    );

    let mut sorted: Vec<&str> = FROZEN.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        FROZEN.len(),
        "duplicate tag string in the frozen set"
    );

    for tag in FROZEN {
        assert!(!tag.is_empty(), "empty tag");
        assert!(
            tag.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "tag must be lowercase snake_case: {tag:?}"
        );
    }

    // The dynamic-prefix builders pin the `": "` separator (consumers do `starts_with(PREFIX)`).
    assert_eq!(r::signer_error("boom"), "signer_error: boom");
    assert_eq!(
        r::broadcast_failed("connection refused"),
        "broadcast_failed: connection refused"
    );
    assert_eq!(r::railgun_keys("x"), "railgun_keys: x");
    assert_eq!(r::sign_failed("x"), "sign_failed: x");
}

// ── Self-tests: prove the scan catches each bypass class and accepts the legit forms. ──

#[test]
fn scan_rejects_bypasses() {
    let cases = [
        // raw literal (incl. one with `//` inside — the masker must not let it evade)
        r#"fn f() { Decision::Deny { reason: "new_tag".into() } }"#,
        r#"fn f() { Decision::Deny { reason: "http://evil".into() } }"#,
        // foreign variable / const
        r#"fn f() { let bad = x(); Decision::Deny { reason: bad.into() } }"#,
        r#"fn f() { ExecuteResult::Denied { reason: format!("forged: {x}") } }"#,
        // foreign const passed to reply_error, and a foreign &reason-prefixed var
        r#"fn f() { reply_error(&mut s, BAD) }"#,
        r#"fn f() { reply_error(&mut s, &reason_code) }"#,
    ];
    for (i, src) in cases.iter().enumerate() {
        let mut v = Vec::new();
        scan_source("case", src, &mut v);
        assert_eq!(
            v.len(),
            1,
            "case {i} should flag exactly one violation, got {v:?}"
        );
    }
}

#[test]
fn scan_accepts_legit_and_skips_tests() {
    let cases = [
        r#"fn f() { Decision::Deny { reason: deny_reasons::REVOKED.into() } }"#,
        r#"fn f() { ExecuteResult::Denied { reason: deny_reasons::signer_error(one_line(&e)) } }"#,
        r#"fn f() { Decision::Deny { reason: reason.clone() } }"#,
        r#"fn f() { let _ = reply_error(&mut s, deny_reasons::MALFORMED_REQUEST); }"#,
        // a Deny mentioned in a line comment must be ignored
        "fn f() {} // Decision::Deny { reason: \"x\" }",
        // a Deny pattern (match arm), not a construction
        "fn f() { match d { Decision::Deny { reason } => g(reason) } }",
        // a test-module literal is skipped, but production code AFTER it is still scanned
        r#"#[cfg(test)]
mod tests {
    fn t() { Decision::Deny { reason: "raw_in_test".into() } }
}
fn prod() { Decision::Deny { reason: deny_reasons::LOCKED.into() } }"#,
    ];
    for (i, src) in cases.iter().enumerate() {
        let mut v = Vec::new();
        scan_source("case", src, &mut v);
        assert!(v.is_empty(), "case {i} should be clean, got {v:?}");
    }
}

#[test]
fn mask_blanks_strings_and_comments() {
    let m = mask("a // b");
    assert_eq!(m.len(), "a // b".len(), "masking preserves length");
    assert_eq!(&m[..2], "a ");
    assert!(
        m[2..].bytes().all(|b| b == b' '),
        "the // comment is blanked: {m:?}"
    );

    // a `//` inside a string must NOT start a comment — code after the string survives
    let m = mask(r#"let s = "x//y"; ok"#);
    assert!(
        m.contains("ok"),
        "code after a //-bearing string must survive: {m:?}"
    );
    assert!(m.contains('"'), "string delimiters are preserved: {m:?}");
    assert!(
        !m.contains("//"),
        "the // inside the string is blanked: {m:?}"
    );

    // braces inside a string are neutralised so brace-balancing stays correct
    let m = mask(r#"f("{a}")"#);
    assert_eq!(m, r#"f("   ")"#);
}

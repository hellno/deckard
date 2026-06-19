//! The mcp.v0.1 acceptance suite — spec `docs/build/30-mcp-shape.md` T1–T4 + T6–T9, as
//! amended by the launch plan: T1 asserts exactly the **6** launch tools (raw `propose` and
//! `simulate` cut); T3/T4 run through an over-cap `shield`; T5 (simulate) dropped; T9 is a
//! structural allowlist walk over the FULL transcript plus a seeded canary; T7 also covers
//! secret-bearing env. Mock daemon (shared `evaluate`, deterministic tx hash), real
//! `deckard-mcp --mcp` child over real stdio JSON-RPC, hard timeouts on every read.

mod common;

use common::*;

/// One fully-wired session: temp dirs, mock daemon, spawned MCP child.
async fn session(
    extra_env: &[(&str, &str)],
) -> (
    TempDir,
    std::sync::Arc<std::sync::Mutex<MockState>>,
    McpChild,
) {
    let dir = TempDir::new("session");
    let socket = dir.path().join("signerd.sock");
    let state = spawn_mock_daemon(&socket);
    // A vault file exists in this config dir, so a locked daemon maps to "locked" (the
    // no-vault error path is asserted separately in the unit tests).
    std::fs::write(dir.path().join("vault.bin"), b"sealed-stand-in").expect("write vault marker");
    let child = McpChild::spawn(&socket, dir.path(), extra_env).await;
    (dir, state, child)
}

/// T1 — `list_tools` is exactly the 6-tool launch profile, every description non-empty and
/// keyword-bearing (the descriptions ARE the agent's documentation).
#[tokio::test]
async fn t1_list_tools_is_the_six_tool_launch_profile() {
    let (_dir, _state, mut child) = session(&[]).await;
    let tools = child.list_tools().await;

    let mut names: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("tool name").to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "deckard_execute",
            "deckard_policy_get",
            "deckard_revoke_all",
            "deckard_shield",
            "deckard_wallet_address",
            "deckard_wallet_balance",
        ],
        "the launch surface is FINAL at these 6 deckard_-prefixed tools"
    );

    // Keyword-bearing descriptions: units, preconditions, sequencing, safety notes.
    let keyword_map: &[(&str, &[&str])] = &[
        ("deckard_wallet_address", &["unlocked", "key-less"]),
        (
            "deckard_wallet_balance",
            &["wei", "Deckard app", "unlocked"],
        ),
        ("deckard_policy_get", &["cap", "FIRST"]),
        (
            "deckard_shield",
            &[
                "decimal ETH string",
                "request_id",
                "deckard_execute",
                "Approvals queue",
            ],
        ),
        (
            "deckard_execute",
            &["tx_hash", "do NOT retry", "already_executed"],
        ),
        ("deckard_revoke_all", &["STOP", "unlock", "Irreversible"]),
    ];
    for (name, keywords) in keyword_map {
        let tool = tools
            .iter()
            .find(|t| t["name"] == *name)
            .unwrap_or_else(|| panic!("{name} missing"));
        let desc = tool["description"].as_str().expect("description");
        assert!(!desc.trim().is_empty(), "{name}: empty description");
        for kw in *keywords {
            assert!(
                desc.contains(kw),
                "{name}: description lost the {kw:?} keyword — it is the agent's docs"
            );
        }
    }

    child.shutdown().await;
}

/// T2 — the read tools succeed with no approval, and the balance is honest about the
/// shielded side (the documented string, never a fake 0).
#[tokio::test]
async fn t2_reads_succeed_and_balance_is_honest_about_shielded() {
    let (_dir, _state, mut child) = session(&[]).await;

    let (err, text, _) = child
        .call_tool("deckard_wallet_address", serde_json::json!({}))
        .await;
    assert!(!err, "address must succeed: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("address JSON");
    assert_eq!(v["address"], format!("{:#x}", mock_address()));

    let (err, text, _) = child
        .call_tool("deckard_wallet_balance", serde_json::json!({}))
        .await;
    assert!(!err, "balance must succeed: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("balance JSON");
    assert_eq!(v["public_eth"], "1");
    assert_eq!(
        v["shielded"], "unavailable — read it in the Deckard app (v1 limitation)",
        "shielded must be the honest unavailable string — '0' would contradict the \
         on-screen hero"
    );
    assert!(v["read_status"]
        .as_str()
        .expect("read_status")
        .contains("unsynced"));

    let (err, text, _) = child
        .call_tool("deckard_policy_get", serde_json::json!({}))
        .await;
    assert!(!err, "policy_get must succeed: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("policy JSON");
    assert_eq!(v["per_tx_cap_eth"], "0.05");
    assert_eq!(v["require_approval"], "over_cap");
    assert_eq!(v["revoked"], false);

    // No secret material in any read response (the transcript-wide walk is T9; this is the
    // per-call assert from the spec).
    let findings = scan_transcript(&child.transcript, &[]);
    assert!(findings.is_empty(), "T2 transcript not clean: {findings:?}");

    child.shutdown().await;
}

/// T3 + T4 — an over-cap shield classifies NeedsApproval (never Allow), carries actionable
/// next-step text, and executing it BEFORE any approval is denied with the three-part
/// catalog error.
#[tokio::test]
async fn t3_t4_overcap_shield_needs_approval_and_early_execute_is_denied() {
    let (_dir, _state, mut child) = session(&[]).await;

    // 0.2 ETH > the 0.05 per-tx cap.
    let (err, text, _) = child
        .call_tool("deckard_shield", serde_json::json!({ "amount_eth": "0.2" }))
        .await;
    assert!(
        !err,
        "an over-cap shield is a decision, not a transport error: {text}"
    );
    let v: serde_json::Value = serde_json::from_str(&text).expect("shield JSON");
    assert_eq!(
        v["decision"], "needs_approval",
        "over-cap must NOT be allow"
    );
    let request_id = v["request_id"].as_str().expect("request_id").to_string();
    let next = v["next"].as_str().expect("actionable next text");
    assert!(
        next.contains("per-tx cap") && next.contains("policy.json"),
        "over-cap copy must say how to get under the fence: {next}"
    );

    // T4: execute before approval → denied, with problem+cause+fix.
    let (err, text, _) = child
        .call_tool(
            "deckard_execute",
            serde_json::json!({ "request_id": request_id }),
        )
        .await;
    assert!(err, "executing an unapproved request must be an error");
    let v: serde_json::Value = serde_json::from_str(&text).expect("error JSON");
    let e = &v["error"];
    assert!(e["problem"].as_str().expect("problem").contains("approval"));
    assert!(!e["cause"].as_str().expect("cause").is_empty());
    assert!(!e["fix"].as_str().expect("fix").is_empty());

    child.shutdown().await;
}

/// T6 — a within-cap shield is allowed and executes to a tx hash (the happy demo path).
#[tokio::test]
async fn t6_within_cap_shield_allows_then_executes_to_tx_hash() {
    let (_dir, _state, mut child) = session(&[]).await;

    let (err, text, _) = child
        .call_tool(
            "deckard_shield",
            serde_json::json!({ "amount_eth": "0.02" }),
        )
        .await;
    assert!(!err, "within-cap shield must be allowed: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("shield JSON");
    assert_eq!(v["decision"], "allow");
    let request_id = v["request_id"].as_str().expect("request_id").to_string();
    assert!(v["next"]
        .as_str()
        .expect("next")
        .contains("deckard_execute"));

    let (err, text, _) = child
        .call_tool(
            "deckard_execute",
            serde_json::json!({ "request_id": request_id }),
        )
        .await;
    assert!(!err, "execute on an allow must broadcast: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("execute JSON");
    assert_eq!(v["status"], "broadcast");
    assert_eq!(v["tx_hash"], format!("{:#x}", mock_tx_hash()));

    // Replay: the same request id is refused with the do-not-retry instruction.
    let (err, text, _) = child
        .call_tool(
            "deckard_execute",
            serde_json::json!({ "request_id": request_id }),
        )
        .await;
    assert!(err, "a replay must be refused");
    assert!(text.contains("already broadcast"), "replay copy: {text}");
    assert!(text.contains("vary the amount"), "replay copy: {text}");

    child.shutdown().await;
}

/// T7 — secret-bearing flags are hard-rejected (value never echoed), and a secret-bearing
/// env var inherited from the host config never reaches a tool response (asserted via the
/// transcript walk in t9; here we pin the flag rejection for both modes).
#[tokio::test]
async fn t7_secret_flags_hard_rejected_without_echo() {
    for argv in [
        vec!["--mcp", "--passphrase=hunter2-super-secret"],
        vec!["balance", "--key=deadbeef-secret"],
    ] {
        let out = tokio::time::timeout(
            IO_TIMEOUT,
            tokio::process::Command::new(env!("CARGO_BIN_EXE_deckard-mcp"))
                .args(&argv)
                .env_clear()
                .output(),
        )
        .await
        .expect("HARD TIMEOUT: secret-flag rejection must be instant")
        .expect("run deckard-mcp");
        assert!(!out.status.success(), "{argv:?} must be rejected");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stderr.contains("secrets are not accepted"),
            "refusal copy missing: {stderr}"
        );
        for secret in ["hunter2-super-secret", "deadbeef-secret"] {
            assert!(
                !stderr.contains(secret) && !stdout.contains(secret),
                "the rejected value was echoed"
            );
        }
    }
}

/// T8 — STOP: revoke_all succeeds, and the next execute is denied with the catalog's
/// revoked entry (re-arm = a human re-unlocks in the app).
#[tokio::test]
async fn t8_stop_then_execute_is_denied() {
    let (_dir, _state, mut child) = session(&[]).await;

    // Arm a within-cap allow first, then STOP before executing it.
    let (_, text, _) = child
        .call_tool(
            "deckard_shield",
            serde_json::json!({ "amount_eth": "0.01" }),
        )
        .await;
    let v: serde_json::Value = serde_json::from_str(&text).expect("shield JSON");
    let request_id = v["request_id"].as_str().expect("request_id").to_string();

    let (err, text, _) = child
        .call_tool("deckard_revoke_all", serde_json::json!({}))
        .await;
    assert!(!err, "STOP must always succeed: {text}");
    assert!(text.contains("stopped"));

    let (err, text, _) = child
        .call_tool(
            "deckard_execute",
            serde_json::json!({ "request_id": request_id }),
        )
        .await;
    assert!(err, "execute after STOP must be denied");
    let v: serde_json::Value = serde_json::from_str(&text).expect("error JSON");
    assert!(
        v["error"]["problem"]
            .as_str()
            .expect("problem")
            .contains("STOP"),
        "{text}"
    );
    assert!(
        v["error"]["fix"]
            .as_str()
            .expect("fix")
            .contains("re-unlock"),
        "{text}"
    );

    child.shutdown().await;
}

/// The connect-time chain probe: a sidecar configured for the wrong chain gets an
/// actionable install-pointing error instead of a confusing late deny.
#[tokio::test]
async fn wrong_chain_daemon_yields_actionable_error() {
    let dir = TempDir::new("wrong-chain");
    let socket = dir.path().join("signerd.sock");
    let _state = spawn_mock_daemon(&socket); // mock signs for MOCK_CHAIN (Sepolia)
    std::fs::write(dir.path().join("vault.bin"), b"sealed-stand-in").expect("vault marker");

    // Point the child at chain 1 — the probe must catch the mismatch on the first call.
    let mut child = McpChild::spawn(&socket, dir.path(), &[("DECKARD_CHAIN_ID", "1")]).await;
    let (err, text, _) = child
        .call_tool("deckard_wallet_address", serde_json::json!({}))
        .await;
    assert!(err, "wrong-chain attach must fail loudly: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("error JSON");
    assert!(
        v["error"]["fix"]
            .as_str()
            .expect("fix")
            .contains("install --demo"),
        "the fix must point at the install helper: {text}"
    );
    child.shutdown().await;
}

/// T9 — the load-bearing assertion: walk the ENTIRE transcript of a full demo session
/// (reads → over-cap → within-cap shield → execute → replay-deny → STOP → post-STOP deny)
/// structurally; permit hex only in the known schema fields; flag secret vocabulary,
/// stray 64-hex, key-in-URL, and high-entropy tokens anywhere else — including a poisoned
/// secret-bearing env var (T7 extension) and the mock's viewing key (never in transcript).
#[tokio::test]
async fn t9_full_session_transcript_is_secret_free() {
    const ENV_CANARY: &str = "EnvCanary12345678901234567890XYZ";
    let poisoned_rpc = format!("https://user:{ENV_CANARY}@rpc.example.com/v3/{ENV_CANARY}");
    let (_dir, state, mut child) = session(&[("DECKARD_RPC_URL", poisoned_rpc.as_str())]).await;

    // A representative full session (T1..T8 shapes in one transcript).
    let _ = child.list_tools().await;
    let _ = child
        .call_tool("deckard_wallet_address", serde_json::json!({}))
        .await;
    let _ = child
        .call_tool("deckard_wallet_balance", serde_json::json!({}))
        .await;
    let _ = child
        .call_tool("deckard_policy_get", serde_json::json!({}))
        .await;
    let (_, over_text, _) = child
        .call_tool("deckard_shield", serde_json::json!({ "amount_eth": "0.2" }))
        .await;
    let over: serde_json::Value = serde_json::from_str(&over_text).expect("shield JSON");
    let _ = child
        .call_tool(
            "deckard_execute",
            serde_json::json!({ "request_id": over["request_id"] }),
        )
        .await;
    let (_, within_text, _) = child
        .call_tool(
            "deckard_shield",
            serde_json::json!({ "amount_eth": "0.02" }),
        )
        .await;
    let within: serde_json::Value = serde_json::from_str(&within_text).expect("shield JSON");
    let id = within["request_id"]
        .as_str()
        .expect("request_id")
        .to_string();
    let _ = child
        .call_tool("deckard_execute", serde_json::json!({ "request_id": &id }))
        .await;
    let _ = child
        .call_tool("deckard_execute", serde_json::json!({ "request_id": &id }))
        .await; // replay deny
    let _ = child
        .call_tool("deckard_revoke_all", serde_json::json!({}))
        .await;
    let _ = child
        .call_tool("deckard_execute", serde_json::json!({ "request_id": &id }))
        .await; // post-STOP deny

    let stderr = child.stderr.lock().expect("stderr lock").clone();
    let findings = scan_transcript(&child.transcript, &stderr);
    assert!(
        findings.is_empty(),
        "secret-shaped material in the transcript: {findings:#?}"
    );

    // Belt and suspenders on the two named canaries (the env secret and the viewing key).
    let all = child.transcript_text();
    assert!(
        !all.contains(ENV_CANARY),
        "the env secret leaked into the transcript"
    );
    let viewing_key = state.lock().expect("state lock").grant.1.clone();
    assert!(
        !all.contains(&viewing_key),
        "the Railgun viewing key leaked into the transcript"
    );

    child.shutdown().await;
}

/// T9 seeded canary — prove the scanner CATCHES a leak: a daemon that fails to redact a
/// broadcast error (fake API key in the reason) must be flagged by the allowlist walk.
/// (That the REAL daemon redacts is pinned in deckard-signerd's guardrail tests.)
#[tokio::test]
async fn t9_seeded_canary_is_caught_by_the_scanner() {
    const CANARY: &str = "FakeKeyCanary0123456789abcdefABCDEF";
    let (_dir, state, mut child) = session(&[]).await;
    state.lock().expect("state lock").force_broadcast_error = Some(format!(
        "broadcast: error sending request for url (https://eth.example.com/v3/{CANARY})"
    ));

    let (_, text, _) = child
        .call_tool(
            "deckard_shield",
            serde_json::json!({ "amount_eth": "0.02" }),
        )
        .await;
    let v: serde_json::Value = serde_json::from_str(&text).expect("shield JSON");
    let id = v["request_id"].as_str().expect("request_id").to_string();
    let (err, _, _) = child
        .call_tool("deckard_execute", serde_json::json!({ "request_id": id }))
        .await;
    assert!(err, "the forced broadcast error must surface");

    let findings = scan_transcript(&child.transcript, &[]);
    assert!(
        !findings.is_empty(),
        "the scanner MUST flag the planted key — otherwise T9 proves nothing"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("URL") || f.contains("high-entropy")),
        "expected a URL/high-entropy finding, got: {findings:?}"
    );

    child.shutdown().await;
}

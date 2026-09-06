//! Canonical probe-cycle state machine + multi-party reconcile.
//!
//! **Authority split**
//! - Backend: cycle_status, active window, analysis_terminal, complete_cycle, cool
//! - Frontend: local upload progress, halt cache, STOP_PROBE, cool cache
//!
//! Either side can be wrong or lagging. Reconcile produces **corrections** so
//! runtime drift (e.g. FE still uploading after 410 / cycle_complete) is fixed
//! without relying on a single channel.

use serde_json::{json, Value};

/// Soft probe progress is NOT the same as cycle closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusinessState {
    Probing,
    /// Soft coverage/stop signal; uploads may still be accepted.
    ProbeSoftComplete,
    /// analysis_terminal just true; race before complete_cycle lands.
    AnalysisTerminalActive,
    IdentityCompleteCool,
    IncompleteExpired,
    Purged,
    Inactive,
}

impl BusinessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Probing => "probing",
            Self::ProbeSoftComplete => "probe_soft_complete",
            Self::AnalysisTerminalActive => "analysis_terminal_active",
            Self::IdentityCompleteCool => "identity_complete_cool",
            Self::IncompleteExpired => "incomplete_expired",
            Self::Purged => "purged",
            Self::Inactive => "inactive",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "probing" => Some(Self::Probing),
            "probe_soft_complete" => Some(Self::ProbeSoftComplete),
            "analysis_terminal_active" => Some(Self::AnalysisTerminalActive),
            "identity_complete_cool" | "cool" | "complete" => Some(Self::IdentityCompleteCool),
            "incomplete_expired" | "incomplete_ttl" => Some(Self::IncompleteExpired),
            "purged" | "cycle_purged" => Some(Self::Purged),
            "inactive" => Some(Self::Inactive),
            _ => None,
        }
    }

    /// Identity uploads must stop for this business state.
    pub fn halt_identity_uploads(self) -> bool {
        matches!(
            self,
            Self::IdentityCompleteCool
                | Self::IncompleteExpired
                | Self::Purged
                | Self::Inactive
                | Self::AnalysisTerminalActive
        )
    }
}

/// Inputs derived from store session_window + latest analysis.
#[derive(Debug, Clone, Default)]
pub struct ServerCycleFacts {
    pub session_id: String,
    pub active: bool,
    pub cycle_status: String,
    pub expired_reason: String,
    pub analysis_terminal: bool,
    pub probe_complete: bool,
    pub stop_probe: bool,
    pub coverage_complete: bool,
    pub received_batch_ids: Vec<String>,
}

/// FE-reported local view (optional; empty fields treated as unknown).
#[derive(Debug, Clone, Default)]
pub struct ClientCycleView {
    pub session_id: String,
    pub stop_probe: Option<bool>,
    pub skip_identity: Option<bool>,
    pub halted: Option<bool>,
    pub phase: Option<String>,
    pub local_uploads_done: Option<bool>,
    pub sent_batch_ids: Vec<String>,
    pub last_http_status: Option<u16>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CycleSnapshot {
    pub business_state: BusinessState,
    pub cycle_closed: bool,
    pub halt_uploads: bool,
    pub skip_identity_probe: bool,
    pub accepts_identity_ingest: bool,
    pub probe_complete: bool,
    pub analysis_terminal: bool,
    pub stop_probe: bool,
    pub coverage_complete: bool,
    pub cycle_status: String,
    pub expired_reason: String,
    pub session_active: bool,
}

/// Derive authoritative snapshot from server facts alone.
pub fn derive_server_snapshot(f: &ServerCycleFacts) -> CycleSnapshot {
    let status = if f.cycle_status.is_empty() {
        "active"
    } else {
        f.cycle_status.as_str()
    };
    let reason = f.expired_reason.as_str();
    let closed = !f.active || status == "complete" || status == "purged";
    // Soft terminal flags (may be true on thin analyze) — hard complete needs B10.
    let analysis_terminal = f.analysis_terminal || (f.stop_probe && f.coverage_complete);
    // Primary B10 only — B10x_* must NOT count (false cool / halt blocked B10_hw_curves).
    let has_b10 = f
        .received_batch_ids
        .iter()
        .any(|b| b == "B10_hw_curves" || b == "mid.curves");
    // Halt identity only when cycle closed OR (final analyze AND hard materials present).
    // Thin analysis_terminal must NOT stop B10 uploads (Opera/WebKit sticky failure mode).
    let hard_final = analysis_terminal && has_b10;
    let halt = closed || hard_final;
    // Soft complete without primary B10 must stay "probing" so FE self-heal continues.
    let soft_complete = (f.probe_complete || f.coverage_complete) && has_b10;
    let business_state = if status == "complete" || reason == "cycle_complete" {
        BusinessState::IdentityCompleteCool
    } else if status == "purged" || reason == "cycle_purged" {
        BusinessState::Purged
    } else if reason == "incomplete_ttl" {
        BusinessState::IncompleteExpired
    } else if !f.active {
        BusinessState::Inactive
    } else if analysis_terminal && has_b10 {
        BusinessState::AnalysisTerminalActive
    } else if soft_complete {
        BusinessState::ProbeSoftComplete
    } else {
        BusinessState::Probing
    };
    CycleSnapshot {
        business_state,
        cycle_closed: closed,
        halt_uploads: halt,
        skip_identity_probe: halt || !f.active,
        accepts_identity_ingest: f.active && !halt,
        // Never report probe_complete to FE without primary B10.
        probe_complete: (f.probe_complete || f.coverage_complete || analysis_terminal) && has_b10,
        analysis_terminal: analysis_terminal && has_b10,
        stop_probe: f.stop_probe && has_b10,
        coverage_complete: f.coverage_complete && has_b10,
        cycle_status: status.to_string(),
        expired_reason: reason.to_string(),
        session_active: f.active,
    }
}

/// Whether evaluate-like result should close the cycle (pure; keep in sync with analysis_completes_cycle).
pub fn result_closes_cycle(result: &Value) -> bool {
    crate::analysis_completes_cycle(result)
}

/// Soft probe_complete alone must never imply halt.
pub fn halt_from_analysis_flags(
    analysis_terminal: bool,
    probe_complete: bool,
    cycle_complete: bool,
    cycle_closed: bool,
) -> bool {
    let _ = probe_complete; // intentionally unused: soft signal
    analysis_terminal || cycle_complete || cycle_closed
}

/// Parse FE-facing terminal decision (mirrors upload_queue parseTerminal intent).
pub fn fe_should_halt_from_response(http_status: u16, body: &Value) -> (bool, &'static str) {
    if http_status == 410 {
        return (true, "http_410");
    }
    let cps = body.get("cycle_probe_status").cloned().unwrap_or(json!({}));
    if body.get("halt_uploads").and_then(|v| v.as_bool()).unwrap_or(false)
        || cps.get("halt_uploads").and_then(|v| v.as_bool()).unwrap_or(false)
        || body
            .pointer("/analysis/halt_uploads")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return (true, "halt_uploads");
    }
    if body.get("cycle_closes").and_then(|v| v.as_bool()).unwrap_or(false)
        || body.get("cycle_complete").is_some()
            && !body.get("cycle_complete").unwrap().is_null()
            && body.get("cycle_complete") != Some(&Value::Bool(false))
        || cps.get("cycle_closed").and_then(|v| v.as_bool()).unwrap_or(false)
        || matches!(
            cps.get("cycle_status").and_then(|v| v.as_str()),
            Some("complete") | Some("purged")
        )
    {
        return (true, "cycle_closed");
    }
    if body
        .get("analysis_terminal")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || body
            .pointer("/analysis/analysis_terminal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || cps
            .get("analysis_terminal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return (true, "analysis_terminal");
    }
    // Soft probe_complete alone → do NOT halt
    if body.get("probe_complete").and_then(|v| v.as_bool()).unwrap_or(false)
        && !body.get("halt_uploads").and_then(|v| v.as_bool()).unwrap_or(false)
    {
        return (false, "soft_probe_complete_only");
    }
    let phase = body.get("phase").and_then(|v| v.as_str()).unwrap_or("");
    if phase == "cool"
        || body
            .get("skip_identity_probe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && (phase == "cool"
                || body.get("business_state").and_then(|v| v.as_str())
                    == Some("identity_complete_cool"))
    {
        return (true, "open_cool");
    }
    let err = format!(
        "{} {}",
        body.get("error").and_then(|v| v.as_str()).unwrap_or(""),
        body.get("code").and_then(|v| v.as_str()).unwrap_or("")
    );
    if err.contains("session_expired")
        || err.contains("cycle_complete")
        || err.contains("cycle_purged")
        || err.contains("incomplete_ttl")
    {
        return (true, "error_code");
    }
    (false, "continue")
}

#[derive(Debug, Clone)]
pub struct Correction {
    pub action: String,
    pub reason: String,
    pub authority: String, // "server" | "client_hint" | "mutual"
}

#[derive(Debug, Clone)]
pub struct ReconcileResult {
    pub server: CycleSnapshot,
    pub corrections: Vec<Correction>,
    pub aligned: bool,
    pub fe_should_halt: bool,
    pub fe_should_resume: bool,
    pub be_should_complete_cycle: bool,
    pub note: String,
}

/// Multi-party reconcile: server is authoritative for cycle lifecycle;
/// client local progress can *hint* soft complete but cannot keep uploading after closed.
pub fn reconcile(server_facts: &ServerCycleFacts, client: Option<&ClientCycleView>) -> ReconcileResult {
    let server = derive_server_snapshot(server_facts);
    let mut corrections = Vec::new();
    let mut fe_should_halt = server.halt_uploads;
    let mut fe_should_resume = false;
    let mut be_should_complete_cycle = false;

    if let Some(c) = client {
        // Session id mismatch → FE must rebind
        if !c.session_id.is_empty()
            && !server_facts.session_id.is_empty()
            && c.session_id != server_facts.session_id
        {
            corrections.push(Correction {
                action: "rebind_session_id".into(),
                reason: format!(
                    "client={} server={}",
                    c.session_id, server_facts.session_id
                ),
                authority: "server".into(),
            });
        }

        // Client still uploading / not halted but server closed → force halt
        let client_active = c.halted == Some(false)
            || c.stop_probe == Some(false)
            || (c.halted.is_none() && c.stop_probe.is_none());
        if server.halt_uploads && client_active && c.halted != Some(true) {
            corrections.push(Correction {
                action: "fe_halt_uploads".into(),
                reason: format!(
                    "server business_state={} cycle_status={}",
                    server.business_state.as_str(),
                    server.cycle_status
                ),
                authority: "server".into(),
            });
            fe_should_halt = true;
        }

        // 410 / expired codes on client → always halt even if client forgot
        if c.last_http_status == Some(410)
            || c.last_error_code
                .as_deref()
                .map(|e| {
                    e.contains("session_expired")
                        || e.contains("cycle_complete")
                        || e.contains("cycle_purged")
                        || e.contains("incomplete_ttl")
                })
                .unwrap_or(false)
        {
            if c.halted != Some(true) {
                corrections.push(Correction {
                    action: "fe_halt_uploads".into(),
                    reason: "client_saw_410_or_expired_code".into(),
                    authority: "mutual".into(),
                });
            }
            fe_should_halt = true;
        }

        // Client halted but server still accepts → FE may have false cool cache
        if c.halted == Some(true)
            && server.accepts_identity_ingest
            && c.phase.as_deref() != Some("cool")
        {
            // Only auto-resume if client didn't claim cool phase (cool is legitimate)
            corrections.push(Correction {
                action: "fe_resume_if_same_active_cycle".into(),
                reason: "server_still_active_client_halted_without_cool".into(),
                authority: "server".into(),
            });
            fe_should_resume = true;
            fe_should_halt = false;
        }

        // Client local uploads done + server analysis_terminal but cycle still active
        // → backend should complete (self-heal missed maybe_complete)
        if c.local_uploads_done == Some(true)
            && server.analysis_terminal
            && server.session_active
            && !server.cycle_closed
        {
            corrections.push(Correction {
                action: "be_complete_cycle".into(),
                reason: "analysis_terminal_but_cycle_still_active".into(),
                authority: "server".into(),
            });
            be_should_complete_cycle = true;
            fe_should_halt = true;
        }

        // Soft complete only on client — do not force halt if server still probing
        if c.local_uploads_done == Some(true)
            && !server.halt_uploads
            && matches!(
                server.business_state,
                BusinessState::Probing | BusinessState::ProbeSoftComplete
            )
        {
            corrections.push(Correction {
                action: "fe_mark_local_done_only".into(),
                reason: "local_upload_progress_not_cycle_closed".into(),
                authority: "client_hint".into(),
            });
        }

        // Client thinks complete (cool) but server active incomplete → clear false cool
        if c.phase.as_deref() == Some("cool")
            && c.skip_identity == Some(true)
            && server.accepts_identity_ingest
            && server.cycle_status == "active"
        {
            corrections.push(Correction {
                action: "fe_clear_false_cool".into(),
                reason: "client_cool_but_server_active".into(),
                authority: "server".into(),
            });
            fe_should_resume = true;
            fe_should_halt = false;
        }
    }

    // Dedupe corrections by action (same drift can match multiple rules).
    let mut seen = std::collections::HashSet::new();
    corrections.retain(|c| seen.insert(c.action.clone()));

    let aligned = corrections.is_empty()
        || corrections
            .iter()
            .all(|c| c.action == "fe_mark_local_done_only");

    ReconcileResult {
        server,
        corrections,
        aligned,
        fe_should_halt,
        fe_should_resume,
        be_should_complete_cycle,
        note: if aligned {
            "aligned".into()
        } else {
            "corrections_required".into()
        },
    }
}

pub fn snapshot_to_json(s: &CycleSnapshot, session_id: &str) -> Value {
    json!({
        "algo": "cycle_probe_status_v2",
        "session_id": session_id,
        "cycle_id": session_id,
        "session_active": s.session_active,
        "cycle_status": s.cycle_status,
        "business_state": s.business_state.as_str(),
        "expired_reason": if s.expired_reason.is_empty() { Value::Null } else { json!(s.expired_reason) },
        "accepts_identity_ingest": s.accepts_identity_ingest,
        "halt_uploads": s.halt_uploads,
        "skip_identity_probe": s.skip_identity_probe,
        "probe_complete": s.probe_complete,
        "analysis_terminal": s.analysis_terminal,
        "stop_probe": s.stop_probe,
        "coverage_complete": s.coverage_complete,
        "cycle_closed": s.cycle_closed,
        "channels": ["ingest_response", "analyze_response", "http_410", "open_cool", "fe_local", "analyze_poll", "probe_status_reconcile"],
        "semantics": {
            "analysis_terminal": "stop_probe AND coverage_complete → closes cycle",
            "probe_complete": "soft coverage/stop signal; alone does not halt uploads",
            "halt_uploads": "cycle closed or analysis_terminal",
            "http_410": "require_active_session failed (complete|purged|incomplete_ttl)",
            "open_cool": "vt cool_until; skip identity",
            "reconcile": "POST probe_status with FE view → server corrections"
        }
    })
}

pub fn reconcile_to_json(r: &ReconcileResult, session_id: &str) -> Value {
    let corrections: Vec<Value> = r
        .corrections
        .iter()
        .map(|c| {
            json!({
                "action": c.action,
                "reason": c.reason,
                "authority": c.authority,
            })
        })
        .collect();
    json!({
        "ok": true,
        "session_id": session_id,
        "aligned": r.aligned,
        "note": r.note,
        "fe_should_halt": r.fe_should_halt,
        "fe_should_resume": r.fe_should_resume,
        "be_should_complete_cycle": r.be_should_complete_cycle,
        "corrections": corrections,
        "cycle_probe_status": snapshot_to_json(&r.server, session_id),
        "halt_uploads": r.fe_should_halt,
        "skip_identity_probe": r.fe_should_halt || r.server.skip_identity_probe,
        "business_state": r.server.business_state.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts_active() -> ServerCycleFacts {
        ServerCycleFacts {
            session_id: "cycle_a".into(),
            active: true,
            cycle_status: "active".into(),
            expired_reason: "".into(),
            analysis_terminal: false,
            probe_complete: false,
            stop_probe: false,
            coverage_complete: false,
            received_batch_ids: vec!["B0_bootstrap".into()],
        }
    }

    #[test]
    fn soft_probe_complete_does_not_halt() {
        let mut f = facts_active();
        f.probe_complete = true;
        f.coverage_complete = true;
        // Without primary B10: stay probing (soft complete is not trustworthy).
        let thin = derive_server_snapshot(&f);
        assert!(!thin.halt_uploads, "soft complete must not halt");
        assert!(thin.accepts_identity_ingest);
        assert_eq!(thin.business_state, BusinessState::Probing);
        // With B10: soft-complete state, still no halt.
        f.received_batch_ids = vec!["B0_bootstrap".into(), "B10_hw_curves".into()];
        let s = derive_server_snapshot(&f);
        assert!(!s.halt_uploads, "soft complete must not halt");
        assert!(s.accepts_identity_ingest);
        assert_eq!(s.business_state, BusinessState::ProbeSoftComplete);
        assert!(!halt_from_analysis_flags(false, true, false, false));
    }

    #[test]
    fn analysis_terminal_halts_and_closes_intent() {
        let mut f = facts_active();
        f.analysis_terminal = true;
        // Thin terminal (no B10) must keep probing + accepting uploads.
        let thin = derive_server_snapshot(&f);
        assert!(!thin.halt_uploads);
        assert!(thin.accepts_identity_ingest);
        assert_eq!(thin.business_state, BusinessState::Probing);
        // Hard final: terminal + B10 materials → halt identity path.
        f.received_batch_ids = vec!["B10_hw_curves".into()];
        let s = derive_server_snapshot(&f);
        assert!(s.halt_uploads);
        assert!(!s.accepts_identity_ingest);
        assert_eq!(s.business_state, BusinessState::AnalysisTerminalActive);
    }

    #[test]
    fn cycle_complete_410_semantics() {
        let f = ServerCycleFacts {
            session_id: "cycle_a".into(),
            active: false,
            cycle_status: "complete".into(),
            expired_reason: "cycle_complete".into(),
            ..Default::default()
        };
        let s = derive_server_snapshot(&f);
        assert!(s.cycle_closed);
        assert!(s.halt_uploads);
        assert_eq!(s.business_state, BusinessState::IdentityCompleteCool);
        let (halt, why) = fe_should_halt_from_response(
            410,
            &json!({"ok": false, "code": "cycle_complete", "halt_uploads": true}),
        );
        assert!(halt);
        assert_eq!(why, "http_410");
    }

    #[test]
    fn fe_does_not_halt_on_soft_probe_complete_body() {
        let (halt, why) = fe_should_halt_from_response(
            200,
            &json!({
                "ok": true,
                "probe_complete": true,
                "halt_uploads": false,
                "cycle_probe_status": {"halt_uploads": false, "probe_complete": true}
            }),
        );
        assert!(!halt);
        assert_eq!(why, "soft_probe_complete_only");
    }

    #[test]
    fn reconcile_forces_halt_when_client_still_active_after_complete() {
        let f = ServerCycleFacts {
            session_id: "cycle_a".into(),
            active: false,
            cycle_status: "complete".into(),
            expired_reason: "cycle_complete".into(),
            ..Default::default()
        };
        let client = ClientCycleView {
            session_id: "cycle_a".into(),
            stop_probe: Some(false),
            halted: Some(false),
            ..Default::default()
        };
        let r = reconcile(&f, Some(&client));
        assert!(r.fe_should_halt);
        assert!(!r.aligned);
        assert!(r
            .corrections
            .iter()
            .any(|c| c.action == "fe_halt_uploads"));
    }

    #[test]
    fn reconcile_clears_false_cool_when_server_active() {
        let f = facts_active();
        let client = ClientCycleView {
            session_id: "cycle_a".into(),
            phase: Some("cool".into()),
            skip_identity: Some(true),
            halted: Some(true),
            ..Default::default()
        };
        let r = reconcile(&f, Some(&client));
        assert!(r.fe_should_resume);
        assert!(!r.fe_should_halt);
        assert!(r
            .corrections
            .iter()
            .any(|c| c.action == "fe_clear_false_cool"));
    }

    #[test]
    fn reconcile_be_complete_when_terminal_stuck_active() {
        let mut f = facts_active();
        f.analysis_terminal = true;
        f.stop_probe = true;
        f.coverage_complete = true;
        f.received_batch_ids = vec!["B10_hw_curves".into()];
        let client = ClientCycleView {
            session_id: "cycle_a".into(),
            local_uploads_done: Some(true),
            halted: Some(false),
            ..Default::default()
        };
        let r = reconcile(&f, Some(&client));
        assert!(r.be_should_complete_cycle);
        assert!(r.fe_should_halt);
    }

    #[test]
    fn result_closes_cycle_strict() {
        // Soft terminal alone never closes — need silicon (maximize probe).
        assert!(!result_closes_cycle(&json!({"analysis_terminal": true})));
        assert!(!result_closes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true}
        })));
        let hard = json!({
            "analysis_terminal": true,
            "b10_present": true,
            "digest_path": "real_curves"
        });
        assert!(result_closes_cycle(&hard));
        assert!(result_closes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true},
            "digest_path": "real_curves"
        })));
        // Hard commercial path (178): silicon + commercial + B10x done → complete
        assert!(result_closes_cycle(&json!({
            "commercial_identity_final": true,
            "b10_present": true,
            "digest_path": "real_curves",
            "route_plan": {"b10x_must_land": false}
        })));
        // B10x still required → do not complete
        assert!(!result_closes_cycle(&json!({
            "commercial_identity_final": true,
            "b10_present": true,
            "digest_path": "real_curves",
            "route_plan": {"b10x_must_land": true}
        })));
        assert!(!result_closes_cycle(&json!({"probe_complete": true})));
        assert!(!result_closes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": false}
        })));
        assert!(!result_closes_cycle(&json!({"session_ticket": {"t": 1}})));
    }

    #[test]
    fn incomplete_ttl_and_purged_states() {
        let f = ServerCycleFacts {
            active: false,
            cycle_status: "active".into(),
            expired_reason: "incomplete_ttl".into(),
            ..Default::default()
        };
        assert_eq!(
            derive_server_snapshot(&f).business_state,
            BusinessState::IncompleteExpired
        );
        let f2 = ServerCycleFacts {
            active: false,
            cycle_status: "purged".into(),
            expired_reason: "cycle_purged".into(),
            ..Default::default()
        };
        assert_eq!(derive_server_snapshot(&f2).business_state, BusinessState::Purged);
    }

    #[test]
    fn flow_probing_to_soft_to_terminal_to_cool() {
        // 1 probing
        let mut f = facts_active();
        assert_eq!(derive_server_snapshot(&f).business_state, BusinessState::Probing);
        // 2 soft without B10 stays probing
        f.probe_complete = true;
        f.coverage_complete = true;
        let s2_thin = derive_server_snapshot(&f);
        assert_eq!(s2_thin.business_state, BusinessState::Probing);
        assert!(!s2_thin.halt_uploads);
        // 2b soft with B10
        f.received_batch_ids = vec!["B0_bootstrap".into(), "B10_hw_curves".into()];
        let s2 = derive_server_snapshot(&f);
        assert_eq!(s2.business_state, BusinessState::ProbeSoftComplete);
        assert!(!s2.halt_uploads);
        // 3 terminal without B10: stay probing, accept late packs
        f.received_batch_ids = vec!["B0_bootstrap".into()];
        f.analysis_terminal = true;
        f.stop_probe = true;
        let s3_soft = derive_server_snapshot(&f);
        assert!(!s3_soft.halt_uploads);
        assert_eq!(s3_soft.business_state, BusinessState::Probing);
        // 3b hard terminal with B10 → halt
        f.received_batch_ids = vec!["B10_hw_curves".into()];
        let s3 = derive_server_snapshot(&f);
        assert!(s3.halt_uploads);
        assert_eq!(s3.business_state, BusinessState::AnalysisTerminalActive);
        // 4 complete cool
        f.active = false;
        f.cycle_status = "complete".into();
        f.expired_reason = "cycle_complete".into();
        let s4 = derive_server_snapshot(&f);
        assert_eq!(s4.business_state, BusinessState::IdentityCompleteCool);
        assert!(s4.cycle_closed);
        // FE after 410 must halt, not treat as incomplete
        let (h, _) = fe_should_halt_from_response(410, &json!({"code":"cycle_complete"}));
        assert!(h);
    }
}

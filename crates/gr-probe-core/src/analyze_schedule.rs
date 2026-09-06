//! Brain-owned analyze trigger policy (not per-batch) — v2.
//!
//! Four automatic arms (plus explicit request):
//! 1. **Coverage complete** — probe schedule floors 100% for the active batch
//! 2. **Idle ≥60s** — no new probe upload for the active batch/cycle
//! 3. **First-result timeout ≥180s** — after first upload of the batch without a
//!    successful analysis result
//! 4. **Pre-cold** — before hot materials demote to cold (arm immediate analyze)
//!
//! **Cold→hot** promotion starts a **new batch context**: clocks re-arm (not inherited).
//!
//! Continuous uploads **reset** the idle quiet window (`IdleReset` merge in store).

use serde_json::{json, Value};

/// No new probe upload for this long → analyze due (idle coalescing).
/// Short-visit default 20s (was 60s) so first product lands before bounce; panel override ok.
pub const ANALYZE_IDLE_UPLOAD_MS: i64 = 20_000;

/// Panel override storage — module-level so the setter and the reader share
/// one location (iss/gpt5.5 P1: two function-local statics never connected,
/// so the panel-published idle override never took effect).
static ANALYZE_IDLE_UPLOAD_MS_OVERRIDE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);

fn analyze_idle_upload_ms_rt() -> i64 {
    let x = ANALYZE_IDLE_UPLOAD_MS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if x >= 5_000 {
        x
    } else {
        ANALYZE_IDLE_UPLOAD_MS
    }
}

/// Admin panel / config publish sets this process-wide.
pub fn set_analyze_idle_upload_ms(ms: i64) {
    ANALYZE_IDLE_UPLOAD_MS_OVERRIDE.store(ms.max(5_000), std::sync::atomic::Ordering::Relaxed);
}
/// No successful analysis result for this long since first upload of batch → force analyze.
/// Short-visit: 90s (was 180s) so silent analyze lag cannot burn the whole dwell.
pub const ANALYZE_NO_RESULT_MS: i64 = 90_000;

/// Policy tag shipped in health/ops.
pub const ANALYZE_SCHEDULE_ALGO: &str = "brain_analyze_schedule_v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzeDueReason {
    NotDue,
    CoverageComplete,
    IdleNoUpload,
    NoResultTimeout,
    PreColdDemote,
    ExplicitRequest,
    /// New batch after cold→hot — not an immediate fire; re-arm clocks only.
    BatchRearm,
}

impl AnalyzeDueReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotDue => "not_due",
            Self::CoverageComplete => "coverage_complete",
            Self::IdleNoUpload => "idle_no_upload",
            Self::NoResultTimeout => "no_result_timeout",
            Self::PreColdDemote => "pre_cold_demote",
            Self::ExplicitRequest => "explicit_request",
            Self::BatchRearm => "batch_rearm",
        }
    }

    pub fn should_run(self) -> bool {
        !matches!(self, Self::NotDue | Self::BatchRearm)
    }
}

/// Pure decision: is analyze due *right now* given clocks for the active batch.
pub fn analyze_due_now(
    now_ms: i64,
    last_upload_ms: i64,
    first_upload_ms: i64,
    last_analyze_ok_ms: Option<i64>,
    coverage_complete: bool,
    cycle_active: bool,
    pre_cold: bool,
    explicit: bool,
) -> AnalyzeDueReason {
    if explicit {
        return AnalyzeDueReason::ExplicitRequest;
    }
    if !cycle_active {
        return AnalyzeDueReason::NotDue;
    }
    if pre_cold {
        return AnalyzeDueReason::PreColdDemote;
    }
    if coverage_complete {
        return AnalyzeDueReason::CoverageComplete;
    }
    // First successful analysis missing since first upload of this batch.
    if last_analyze_ok_ms.is_none()
        && first_upload_ms > 0
        && now_ms.saturating_sub(first_upload_ms) >= ANALYZE_NO_RESULT_MS
    {
        return AnalyzeDueReason::NoResultTimeout;
    }
    if last_upload_ms > 0 && now_ms.saturating_sub(last_upload_ms) >= analyze_idle_upload_ms_rt() {
        return AnalyzeDueReason::IdleNoUpload;
    }
    AnalyzeDueReason::NotDue
}

/// Absolute `due_ms` to arm after an ingest for the active batch.
///
/// Always arms **idle** (`now + 60s`). Also arms **first-result ceiling** at
/// `first_upload + 180s` when no analysis exists yet. Coverage complete → now.
pub fn analyze_arm_due_ms_after_ingest(
    now_ms: i64,
    first_upload_ms: i64,
    last_analyze_ok_ms: Option<i64>,
    coverage_complete: bool,
) -> i64 {
    if coverage_complete {
        return now_ms;
    }
    let idle_due = now_ms.saturating_add(analyze_idle_upload_ms_rt());
    if last_analyze_ok_ms.is_none() && first_upload_ms > 0 {
        let no_result_due = first_upload_ms.saturating_add(ANALYZE_NO_RESULT_MS);
        return idle_due.min(no_result_due);
    }
    idle_due
}

/// Debounce ms relative to now for `schedule_analyze` compatibility.
pub fn analyze_arm_debounce_ms_after_ingest(
    now_ms: i64,
    first_upload_ms: i64,
    last_analyze_ok_ms: Option<i64>,
    coverage_complete: bool,
) -> i64 {
    let due = analyze_arm_due_ms_after_ingest(
        now_ms,
        first_upload_ms,
        last_analyze_ok_ms,
        coverage_complete,
    );
    (due - now_ms).max(0)
}

/// Pre-cold demote arm: immediate analyze due.
pub fn analyze_arm_due_ms_pre_cold(now_ms: i64) -> i64 {
    now_ms
}

/// Cold→hot / new batch: return a fresh clock epoch for re-arm (first_upload = now).
pub fn new_batch_analyze_clocks(now_ms: i64) -> Value {
    json!({
        "batch_epoch_ms": now_ms,
        "first_upload_ms": 0,
        "last_upload_ms": 0,
        "last_analyze_ok_ms": null,
        "reason": AnalyzeDueReason::BatchRearm.as_str(),
        "rearm": true,
    })
}

/// Ops / health projection of the policy constants.
pub fn analyze_schedule_policy_json() -> Value {
    json!({
        "algo": ANALYZE_SCHEDULE_ALGO,
        "idle_upload_ms": analyze_idle_upload_ms_rt(),
        "no_result_ms": ANALYZE_NO_RESULT_MS,
        "per_batch_analyze": false,
        "schedule_from_analysis_result": false,
        "triggers": [
            "coverage_complete_100",
            "idle_no_upload",
            "no_result_90s_since_first_upload",
            "pre_cold_demote",
            "explicit_request"
        ],
        "cold_to_hot_rearms_batch_clocks": true,
        "cold_hot_owner": "brain",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iss/gpt5.5 P1 regression: the panel-published idle override must reach
    /// the reader (previously setter/getter each had their own static).
    #[test]
    fn panel_idle_override_reaches_reader() {
        let prev = ANALYZE_IDLE_UPLOAD_MS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        set_analyze_idle_upload_ms(45_000);
        assert_eq!(analyze_idle_upload_ms_rt(), 45_000);
        // Below-floor values clamp to the 5s floor, never disable the arm.
        set_analyze_idle_upload_ms(1);
        assert_eq!(analyze_idle_upload_ms_rt(), 5_000);
        // Restore so other tests observe the default.
        ANALYZE_IDLE_UPLOAD_MS_OVERRIDE.store(prev, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn ingest_burst_does_not_force_immediate_analyze() {
        let now = 1_000_000_i64;
        let due = analyze_arm_due_ms_after_ingest(now, now - 5_000, None, false);
        assert!(due > now, "must not be immediate on incomplete coverage");
        assert_eq!(due, now + ANALYZE_IDLE_UPLOAD_MS);
    }

    #[test]
    fn coverage_complete_is_immediate() {
        let now = 2_000_000_i64;
        let due = analyze_arm_due_ms_after_ingest(now, now - 10_000, None, true);
        assert_eq!(due, now);
        assert_eq!(
            analyze_due_now(now, now, now - 10_000, None, true, true, false, false),
            AnalyzeDueReason::CoverageComplete
        );
    }

    #[test]
    fn idle_upload_triggers() {
        let first = 0_i64;
        let last_up = 100_000_i64;
        let now = last_up + ANALYZE_IDLE_UPLOAD_MS;
        assert_eq!(
            analyze_due_now(now, last_up, first, Some(50_000), false, true, false, false),
            AnalyzeDueReason::IdleNoUpload
        );
        // Half idle window — still not due
        let half = (ANALYZE_IDLE_UPLOAD_MS / 2).max(1);
        assert_eq!(
            analyze_due_now(
                last_up + half,
                last_up,
                first,
                Some(50_000),
                false,
                true,
                false,
                false
            ),
            AnalyzeDueReason::NotDue
        );
    }

    #[test]
    fn no_result_180s_from_first_upload() {
        let first = 1_000_000_i64;
        let now = first + ANALYZE_NO_RESULT_MS;
        // Recent upload — idle not due, but no-result from first upload is
        assert_eq!(
            analyze_due_now(now, now, first, None, false, true, false, false),
            AnalyzeDueReason::NoResultTimeout
        );
    }

    #[test]
    fn pre_cold_triggers() {
        assert_eq!(
            analyze_due_now(100, 100, 50, Some(80), false, true, true, false),
            AnalyzeDueReason::PreColdDemote
        );
        assert_eq!(analyze_arm_due_ms_pre_cold(100), 100);
    }

    #[test]
    fn cold_hot_rearm_clocks() {
        let c = new_batch_analyze_clocks(999);
        assert_eq!(c["batch_epoch_ms"], 999);
        assert_eq!(c["rearm"], true);
        assert!(c["last_analyze_ok_ms"].is_null());
    }

    #[test]
    fn no_result_ceiling_caps_arm_due() {
        let first = 1_000_000_i64;
        let now = first + 170_000;
        let due = analyze_arm_due_ms_after_ingest(now, first, None, false);
        assert_eq!(due, first + ANALYZE_NO_RESULT_MS);
    }

    #[test]
    fn inactive_cycle_not_due() {
        assert_eq!(
            analyze_due_now(999_999, 0, 0, None, true, false, false, false),
            AnalyzeDueReason::NotDue
        );
    }
}

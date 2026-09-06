//! In-tree component catalog: pack_id ↔ FE runtime, static vs dynamic schedule.

use crate::contracts::{find_spec_dir, ContractError};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct PackDef {
    pub pack_id: String,
    pub aliases: Vec<String>,
    pub batch_id: String,
    pub layer: String,
    pub schedule: String,
    pub priority: i64,
    pub source: String,
    pub executable: bool,
    pub hard_eligible: bool,
    pub requires_soft_v2: bool,
    /// Explicit engine-family allowlist (FE gates each pack by classified
    /// engine before running: pack_loader engine_profile_excluded). Empty =
    /// fall back to the id-based heuristic in probe_dag_v2. SSOT lives in
    /// spec/component_catalog.json `engines`; cross-checked against
    /// spec/probe_fallback_priority.json engine_profiles capability flags.
    pub engines: Vec<String>,
    /// B-batch four-piece gate (audit report §4.2 / "新 B 批四件套门控模板"):
    /// - `research` — 研究/诊断权; excluded from default scheduling until the
    ///   dual-KPI gate (intra-device + inter-device stability) clears.
    /// - `default` — admitted into default scheduling; promotion to *unique
    ///   silicon claim* is still gated by the intra/inter-device KPIs.
    pub gate: String,
}

#[derive(Debug, Clone)]
pub struct ComponentCatalog {
    pub version: String,
    pub packs: Vec<PackDef>,
    pub by_id: HashMap<String, PackDef>,
    pub static_kick_order: Vec<String>,
}

impl ComponentCatalog {
    pub fn load_from(spec_dir: &Path) -> Result<Self, ContractError> {
        let path = spec_dir.join("component_catalog.json");
        let text = fs::read_to_string(&path).map_err(|e| {
            ContractError::new(format!("missing catalog {}: {e}", path.display()))
        })?;
        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| ContractError::new(format!("invalid catalog json: {e}")))?;
        Self::from_value(&raw)
    }

    pub fn from_value(raw: &Value) -> Result<Self, ContractError> {
        let version = raw
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("v5.1")
            .to_string();
        let arr = raw
            .get("packs")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ContractError::new("catalog.packs must be array"))?;
        let mut packs = Vec::new();
        let mut by_id = HashMap::new();
        for (i, p) in arr.iter().enumerate() {
            let pack_id = p
                .get("pack_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ContractError::new(format!("packs[{i}].pack_id required")))?
                .to_string();
            let aliases: Vec<String> = p
                .get("aliases")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            let batch_id = p
                .get("batch_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&pack_id)
                .to_string();
            let layer = p
                .get("layer")
                .and_then(|v| v.as_str())
                .unwrap_or("lite5")
                .to_string();
            let schedule = p
                .get("schedule")
                .and_then(|v| v.as_str())
                .unwrap_or("static")
                .to_string();
            let priority = p
                .get("priority")
                .and_then(|v| v.as_i64())
                .unwrap_or(50);
            let source = p
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            let executable = p
                .get("executable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let hard_eligible = p
                .get("hard_eligible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let requires_soft_v2 = p
                .get("requires_soft_v2")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let engines: Vec<String> = p
                .get("engines")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            // B-batch gate: explicit spec overrides; otherwise research-lane packs
            // (B18 webgpu, B47 SAB) stay research-gated by naming convention.
            let gate = p
                .get("gate")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    if pack_id.starts_with("B18_") || pack_id.starts_with("B47_") {
                        "research".into()
                    } else {
                        "default".into()
                    }
                });
            let def = PackDef {
                pack_id: pack_id.clone(),
                aliases: aliases.clone(),
                batch_id,
                layer,
                schedule,
                priority,
                source,
                executable,
                hard_eligible,
                requires_soft_v2,
                engines,
                gate,
            };
            by_id.insert(pack_id.clone(), def.clone());
            for a in aliases {
                by_id.entry(a).or_insert_with(|| def.clone());
            }
            packs.push(def);
        }
        let static_kick_order: Vec<String> = raw
            .get("static_kick_order")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        Ok(Self {
            version,
            packs,
            by_id,
            static_kick_order,
        })
    }

    /// Resolve pack_id or alias → canonical PackDef.
    pub fn resolve(&self, id: &str) -> Option<&PackDef> {
        self.by_id.get(id)
    }

    pub fn static_packs(&self) -> Vec<&PackDef> {
        let order = &self.static_kick_order;
        if !order.is_empty() {
            return order
                .iter()
                .filter_map(|id| self.by_id.get(id.as_str()))
                .filter(|p| p.schedule == "static" && p.executable && p.gate != "research")
                .collect();
        }
        self.packs
            .iter()
            .filter(|p| p.schedule == "static" && p.executable && p.gate != "research")
            .collect()
    }

    pub fn executable_rate(&self) -> f64 {
        let n = self.packs.len().max(1) as f64;
        let ok = self.packs.iter().filter(|p| p.executable).count() as f64;
        ok / n
    }
}

impl PackDef {
    /// Machine-readable probe DAG v2 metadata (defaults from pack/layer/source).
    ///
    /// iss/design §9.3: every pack carries planes/depends_on/resource/deadline_ms/
    /// engine_profiles/cost_class/commercial_roles/diagnostic_roles/fallbacks/
    /// missing_state/replay_binding. FE consumes these fields to schedule
    /// (see pack_loader.js dag_v2 pass); the same metadata is exposed in the
    /// route_plan so "why was this field not executed" is answerable from the
    /// sealed pack-set decision (engine claim + dag version).
    pub fn probe_dag_v2(&self) -> serde_json::Value {
        use serde_json::json;
        let id = self.pack_id.as_str();
        let planes: Vec<&str> = if self.source == "gateway" || self.source == "cloudflare" {
            vec!["S0"]
        } else if id.contains("B11") {
            vec!["S3"]
        } else if id.contains("B7") {
            vec!["S2"]
        } else if id.contains("B10") {
            vec!["S1", "S2"]
        } else if id.contains("B18") {
            vec!["S1"]
        } else {
            vec!["S1"]
        };
        let cost = if id.contains("B10") || id.contains("B18") {
            "heavy"
        } else if id.contains("B7") || id.contains("B11") {
            "medium"
        } else {
            "light"
        };
        // Soft deadline per cost class; deepen lanes get shorter ceilings so the
        // session can degrade instead of burning the whole window (self-heal
        // timeouts already mirror these values).
        let deadline_ms = if id.contains("B10x_") {
            30_000
        } else if id.contains("B10") || id == "mid.curves" {
            90_000
        } else if id.contains("B18") {
            45_000
        } else if cost == "medium" {
            20_000
        } else {
            8_000
        };
        // Engine families a pack may run on (FE gates by classified engine).
        // UA is only a claim; the FE classifies by API surface evidence first.
        // Explicit spec `engines` wins (data-driven SSOT for engine-gated packs
        // like B92/B93/B95); otherwise the id-based heuristic below applies.
        let engine_profiles: Vec<&str> = if !self.engines.is_empty() {
            self.engines.iter().map(|s| s.as_str()).collect()
        } else if id.contains("webkit") {
            vec!["webkit"]
        } else if id.contains("angle_crosscheck") {
            vec!["blink"]
        } else if id.contains("B10x_") || id.contains("B10") || id.contains("B18") {
            vec!["blink", "gecko", "webkit", "android"]
        } else if id.contains("B7") || id.contains("B9") || id.contains("B55") {
            vec!["blink", "gecko", "webkit", "ios", "android", "webview"]
        } else {
            vec!["blink", "gecko", "webkit", "ios", "android", "webview", "vm"]
        };
        // Resource classes mirror the FE ResourceBus normalizer (main ∥ iframe
        // share the bus; compound packs multi-lock).
        let resource: Vec<&str> = if id.contains("B10") || id.contains("B18") {
            vec!["gpu"]
        } else if id.contains("B46") || id.contains("B61") || id.contains("B62") {
            vec!["audio"]
        } else if id.contains("B9") || id.contains("B55") {
            vec!["rtc"]
        } else if id.contains("B7") {
            vec!["nest"]
        } else if id.contains("B2")
            || id.contains("B17")
            || id.contains("B20")
            || id.contains("B33")
            || id.contains("B34")
            || id.contains("B37")
            || id.contains("B42")
            || id.contains("B47")
            || id.contains("B82")
            || id.contains("B83")
        {
            vec!["cpu"]
        } else if id.starts_with('R') || id.contains("spotcheck") {
            vec!["verify"]
        } else {
            vec!["light"]
        };
        // Honest fallback ladder: never brute re-kick the same path (design §11).
        let fallbacks: Vec<&str> = if id == "B10_hw_curves" || id == "mid.curves" {
            vec!["B10x_softgl_hedge", "B10x_legacy_webgl1", "honest_skip"]
        } else if id == "B18_webgpu" {
            vec!["B22_gpu_timer", "B31_shader_numeric", "degraded"]
        } else if id.contains("B10x_silicon_") {
            vec!["B10x_softgl_hedge", "honest_skip"]
        } else {
            vec!["honest_skip"]
        };
        let commercial = if self.hard_eligible {
            json!(["silicon_candidate"])
        } else if self.source == "gateway" || self.source == "cloudflare" {
            json!(["network_axis"])
        } else {
            json!(["supporting"])
        };
        // New B-batch four-piece gate template (§4.2): every pack carries
        // (1) observation boundary, (2) independent failure modes, (3) dual-KPI
        // gate status, (4) upload/storage/analysis cost budget. `research`-gated
        // packs (B18/B47) stay out of default scheduling until the KPI gate clears.
        let (observation_bound, independent_failure_mode): (&str, Vec<&str>) =
            if id.starts_with("B10") {
                (
                    "single-visit residual curve; 32-sample lanes; warm+dual-context; self-heal timeout",
                    vec!["webgl_context_loss", "webgl_driver_override", "gpu_timeout", "entropy_collapse"],
                )
            } else if id.starts_with("B18") {
                (
                    "single-visit WebGPU adapter+limits+dual powerPreference; honest skip when GPU absent",
                    vec!["webgpu_unavailable", "adapter_null", "validation_error", "lab_renderer_shielded"],
                )
            } else if id.starts_with("B47") {
                (
                    "SAB/Atomics clock foundation; honest skip without COOP/COEP isolation",
                    vec!["sab_unavailable", "coop_coep_missing", "timeout_clamp", "ab_slice_empty"],
                )
            } else if id.contains("B11") || id.contains("B12") {
                (
                    "early-bind interaction window; pagehide flush; short-visit doable",
                    vec!["pagehide_flush_lost", "idle_throttle", "early_exit"],
                )
            } else if id == "B8_gateway_early" || id == "edge.cf" {
                (
                    "edge request-bound; single inject path; no FE dependency",
                    vec!["edge_timeout", "cf_edge_unavailable", "inject_miss"],
                )
            } else {
                (
                    "single-visit batch; per-layer deadline cap; honest skip on platform gap",
                    vec!["timeout", "harness_error", "platform_gap"],
                )
            };
        // Upload/storage/analysis cost budget (bytes/rows/class rough estimates).
        let (upload_bytes_est, rows_per_session) = match cost {
            "heavy" => (24_000, 400),
            "medium" => (8_000, 120),
            _ => (2_000, 40),
        };
        json!({
            "pack_id": self.pack_id,
            "batch_id": self.batch_id,
            "planes": planes,
            "depends_on": if id.contains("B10") { json!(["B0_bootstrap"]) } else { json!([]) },
            "resource": resource,
            "deadline_ms": deadline_ms,
            "engine_profiles": engine_profiles,
            "cost_class": cost,
            "source_kind": if self.source == "cloudflare" { "cloud_edge" } else if self.source == "gateway" { "gateway" } else { "fe" },
            "commercial_roles": commercial,
            "diagnostic_roles": ["coverage"],
            "fallbacks": fallbacks,
            "missing_state": "degraded",
            "replay_binding": "challenge_seed",
            // four-piece gate template
            "gate": self.gate,
            "observation_bound": observation_bound,
            "independent_failure_mode": independent_failure_mode,
            "dual_kpi_gate": if self.gate == "research" { "research" } else { "default_admission" },
            "cost_budget": {
                "upload_bytes_est": upload_bytes_est,
                "rows_per_session": rows_per_session,
                "analysis_cost_class": cost,
            },
        })
    }
}

/// New B-batch four-piece gate template validation (§4.2).
///
/// Contract for **new** packs (and a standing invariant for the catalog):
/// every executable pack carries the four gate pieces via `probe_dag_v2`;
/// research-gated packs must never land in `static_kick_order` (no default-high
/// priority), and `dual_kpi_gate` must be consistent with the gate.
pub fn validate_pack_gate_template(cat: &ComponentCatalog) -> Result<(), ContractError> {
    let mut problems: Vec<String> = Vec::new();
    let required = [
        "gate",
        "observation_bound",
        "independent_failure_mode",
        "dual_kpi_gate",
        "cost_budget",
    ];
    for def in &cat.packs {
        if !def.executable {
            continue;
        }
        let dag = def.probe_dag_v2();
        for key in required {
            if dag.get(key).is_none() {
                problems.push(format!("{} missing four-piece key {key}", def.pack_id));
            }
        }
        if def.gate == "research" && cat.static_kick_order.iter().any(|id| id == &def.pack_id) {
            problems.push(format!(
                "{} research-gated but present in static_kick_order",
                def.pack_id
            ));
        }
        if dag.get("dual_kpi_gate").and_then(|v| v.as_str()).is_none() {
            problems.push(format!("{} dual_kpi_gate not a string", def.pack_id));
        }
    }
    // Enum sanity: gate values are closed set.
    for def in &cat.packs {
        if !matches!(def.gate.as_str(), "default" | "research" | "diagnostic") {
            problems.push(format!("{} invalid gate {:?}", def.pack_id, def.gate));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ContractError::new(format!(
            "pack gate template violations: {}",
            problems.join("; ")
        )))
    }
}

static CATALOG: OnceLock<Result<ComponentCatalog, String>> = OnceLock::new();

pub fn load_catalog() -> Result<&'static ComponentCatalog, ContractError> {
    let r = CATALOG.get_or_init(|| {
        let dir = find_spec_dir();
        ComponentCatalog::load_from(&dir).map_err(|e| e.to_string())
    });
    match r {
        Ok(c) => Ok(c),
        Err(e) => Err(ContractError::new(e.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_and_static_resolves() {
        let c = load_catalog().expect("catalog");
        assert!(c.executable_rate() >= 0.95);
        assert!(c.resolve("B0_bootstrap").is_some());
        assert_eq!(
            c.resolve("lite.surface").map(|p| p.pack_id.as_str()),
            Some("B0_bootstrap")
        );
        assert_eq!(
            c.resolve("gateway.b8").map(|p| p.pack_id.as_str()),
            Some("B8_gateway_early")
        );
        assert_eq!(
            c.resolve("mid.curves").map(|p| p.schedule.as_str()),
            Some("static"),
            "B10 hard curves are static high-priority commercial anchors"
        );
        assert_eq!(
            c.resolve("B10_hw_curves").map(|p| p.hard_eligible),
            Some(true)
        );
        let statics: Vec<_> = c.static_packs().iter().map(|p| p.pack_id.as_str()).collect();
        assert!(statics.contains(&"B8_gateway_early"));
        assert!(statics.contains(&"B0_bootstrap"));
        assert!(statics.contains(&"B1_conflict"));
        assert!(
            statics.iter().any(|id| *id == "B10_hw_curves"),
            "B10_hw_curves must be in static_kick for short-visit eligibility"
        );
        let dag = c.resolve("B8_gateway_early").unwrap().probe_dag_v2();
        assert_eq!(dag["source_kind"], "gateway");
        assert_eq!(dag["planes"][0], "S0");
        assert_eq!(dag["commercial_roles"][0], "network_axis");
    }

    #[test]
    fn dag_v2_metadata_complete_for_scheduler() {
        let c = load_catalog().expect("catalog");
        let b10 = c
            .resolve("B10_hw_curves")
            .expect("B10")
            .probe_dag_v2();
        // Scheduler fields every pack must carry.
        for key in [
            "planes",
            "depends_on",
            "resource",
            "deadline_ms",
            "engine_profiles",
            "cost_class",
            "source_kind",
            "commercial_roles",
            "diagnostic_roles",
            "fallbacks",
            "missing_state",
            "replay_binding",
        ] {
            assert!(
                b10.get(key).is_some(),
                "B10 dag_v2 missing {key}: {b10}"
            );
        }
        assert_eq!(b10["cost_class"], "heavy");
        assert_eq!(b10["deadline_ms"], 90_000);
        assert_eq!(b10["depends_on"][0], "B0_bootstrap");
        assert!(b10["engine_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "blink"));
        // WebKit-only lane must not run on Blink (engine gate enforceability).
        let wk = c
            .resolve("B10x_webkit_gl_noise")
            .expect("webkit lane")
            .probe_dag_v2();
        assert_eq!(
            wk["engine_profiles"].as_array().unwrap().as_slice(),
            &[serde_json::Value::String("webkit".into())]
        );
        assert_eq!(wk["deadline_ms"], 30_000);
        // Honest fallback ladder present on the primary residual pack.
        assert_eq!(b10["fallbacks"][0], "B10x_softgl_hedge");
        // Four-piece gate template is complete on every executable pack.
        validate_pack_gate_template(&c).expect("gate template invariant holds");
        assert_eq!(b10["gate"], "default");
        assert_eq!(b10["dual_kpi_gate"], "default_admission");
        assert!(b10["observation_bound"].as_str().unwrap().contains("residual"));
        assert_eq!(
            b10["cost_budget"]["upload_bytes_est"].as_u64().unwrap(),
            24_000
        );
        assert!(
            b10["fallbacks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "honest_skip")
        );
    }

    #[test]
    fn research_gated_packs_stay_out_of_default_schedule() {
        let c = load_catalog().expect("catalog");
        // B18/B47 families are research-gated by convention + explicit spec.
        let b18 = c.resolve("B18_webgpu").expect("B18");
        assert_eq!(b18.gate, "research");
        assert_eq!(b18.probe_dag_v2()["dual_kpi_gate"], "research");
        let b47 = c.resolve("B47_sab_clock").expect("B47");
        assert_eq!(b47.gate, "research");
        let b47_flags = c.resolve("B47_api_flags_detail").expect("B47 flags");
        assert_eq!(b47_flags.gate, "research");
        // Research packs must never appear in static kick order (default high priority).
        for p in [&b18, &b47, &b47_flags] {
            assert!(
                !c.static_kick_order.iter().any(|id| id == &p.pack_id),
                "{} research-gated but in static_kick_order",
                p.pack_id
            );
        }
        // Static default wave excludes research packs.
        let statics: Vec<&str> = c.static_packs().iter().map(|p| p.pack_id.as_str()).collect();
        assert!(!statics.contains(&"B18_webgpu"));
        assert!(!statics.contains(&"B47_sab_clock"));
        // Every static pack is default-gated and carries the four-piece template.
        for p in c.static_packs() {
            assert_eq!(p.gate, "default", "{}", p.pack_id);
            let dag = p.probe_dag_v2();
            assert!(dag.get("observation_bound").is_some(), "{}", p.pack_id);
            assert!(
                dag.get("independent_failure_mode")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty()),
                "{}",
                p.pack_id
            );
        }
        // Gate template validator also rejects a synthetic violation.
        let mut raw = serde_json::from_str::<Value>(
            &fs::read_to_string(find_spec_dir().join("component_catalog.json"))
                .expect("catalog text"),
        )
        .expect("catalog json");
        let mut packs2 = raw["packs"].as_array().cloned().unwrap_or_default();
        packs2.push(serde_json::json!({
            "pack_id": "B99_phantom_research",
            "layer": "deep",
            "schedule": "static",
            "priority": 40,
            "source": "main",
            "gate": "research"
        }));
        raw["packs"] = serde_json::Value::Array(packs2);
        raw["static_kick_order"] = serde_json::json!([
            "B99_phantom_research",
            "B8_gateway_early"
        ]);
        let bad = ComponentCatalog::from_value(&raw).expect("catalog parses");
        let err = validate_pack_gate_template(&bad).expect_err("template must reject");
        assert!(
            err.to_string().contains("research-gated but present in static_kick_order"),
            "{err}"
        );
        // cost_budget present on research lanes too (template complete, schedule-gated).
        assert_eq!(
            b18.probe_dag_v2()["cost_budget"]["analysis_cost_class"],
            "heavy"
        );
    }

    #[test]
    fn b84_gpu_bandwidth_alias_resolves_and_carries_four_piece_gate() {
        // iss/74 batch 9: FE registry registers B84_gpu_bandwidth_ladder as an
        // alias of B30_gpu_bandwidth (shared runtime, batch_id B30). The catalog
        // previously omitted the alias → route plans referencing the ladder id
        // could not resolve. Fixed: alias now maps to the canonical entry.
        let c = load_catalog().expect("catalog");
        let b84 = c.resolve("B84_gpu_bandwidth_ladder").expect("B84 alias resolves");
        assert_eq!(b84.pack_id, "B30_gpu_bandwidth");
        assert_eq!(b84.batch_id, "B30_gpu_bandwidth");
        let dag = b84.probe_dag_v2();
        assert_eq!(dag["pack_id"], "B30_gpu_bandwidth");
        assert_eq!(dag["resource"][0], "light");
        assert!(dag.get("observation_bound").is_some());
        assert_eq!(c.resolve("B30_gpu_bandwidth").expect("B30").pack_id, "B30_gpu_bandwidth");
    }

    #[test]
    fn engine_gated_packs_honor_explicit_engines_in_dag() {
        let c = load_catalog().expect("catalog");
        // B93/B95 are blink-only (UA-CH brand surface / DrawnApart OffscreenCanvas
        // webgl2 EU timing); B92 runs on all three WebGPU families.
        let dag = |id: &str| -> Vec<String> {
            c.resolve(id)
                .expect(id)
                .probe_dag_v2()["engine_profiles"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        };
        assert_eq!(dag("B93_blink_fork_matrix"), vec!["blink"]);
        assert_eq!(dag("B95_gpu_eu_timing"), vec!["blink"]);
        let b92 = dag("B92_webgpu_atomic_contention");
        assert!(b92.contains(&"blink".to_string()));
        assert!(b92.contains(&"gecko".to_string()));
        assert!(b92.contains(&"webkit".to_string()));
        assert_eq!(b92.len(), 3);
        // B94 keeps SAB universal families (explicit engines not set → heuristic).
        let b94 = dag("B94_sab_dual_clock_differential");
        assert!(b94.contains(&"blink".to_string()));
        assert!(b94.contains(&"webkit".to_string()));
    }

    #[test]
    fn engine_gate_matches_fallback_priority_capability_table() {
        // Red line: 新探测一律先过引擎能力表（engine_profiles）再进排期.
        // component_catalog `engines` must be the same allowlist as
        // spec/probe_fallback_priority.json pack_engine_gate, and every listed
        // family must declare the pack's primary capability in its profile.
        let c = load_catalog().expect("catalog");
        let mut gate: serde_json::Value =
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(find_spec_dir().join("probe_fallback_priority.json")).expect("spec"))
                .expect("spec json")
                .get("pack_engine_gate")
                .cloned()
                .unwrap_or_default();
        let profiles: serde_json::Value = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(find_spec_dir().join("probe_fallback_priority.json")).expect("spec"),
        )
        .expect("spec json")
        .get("engine_profiles")
        .cloned()
        .unwrap_or_default();
        for (pack_id, spec) in gate.as_object_mut().unwrap().iter_mut() {
            let engines: Vec<String> = spec
                .get("engines")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            let capability = spec
                .get("capability")
                .and_then(|v| v.as_str())
                .unwrap_or("wasm");
            if let Some(def) = c.resolve(pack_id) {
                let dag_engines: Vec<String> = def.probe_dag_v2()["engine_profiles"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                for e in &engines {
                    assert!(
                        dag_engines.iter().any(|d| d == e),
                        "{pack_id} catalog engines missing {e} (spec gate {engines:?}, dag {dag_engines:?})"
                    );
                }
                for e in &engines {
                    let caps = profiles
                        .get(e)
                        .and_then(|p| p.get("capabilities"))
                        .cloned()
                        .unwrap_or_default();
                    assert!(
                        caps.get(capability).and_then(|v| v.as_bool()).unwrap_or(false),
                        "{pack_id} engine {e} lacks capability {capability} in engine_profiles"
                    );
                }
            }
        }
    }
}

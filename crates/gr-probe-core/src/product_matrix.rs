//! Load field_product_matrix + task_gap_map from spec/ (shared evidence contracts).

use crate::contracts::{find_spec_dir, ContractError};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct FieldAxisRoles {
    pub field: String,
    pub primary_packs: Vec<String>,
    /// axis -> role (material|conf|veto|diag|exclude)
    pub axes: HashMap<String, String>,
    /// G-ARCH-21: T0–T5 trust tier (capabilities…edge).
    pub trust_tier: String,
}

/// F-11 SSOT: commercial device_id digest materials (mirrors trust commercial_projection).
#[derive(Debug, Clone, Default)]
pub struct CommercialDeviceIdSsot {
    pub digest_order: Vec<String>,
    pub hardware_anchors: Vec<String>,
    pub soft_commercial_extra: Vec<String>,
    pub never_digest: Vec<String>,
}

/// iss/25: matrix rows that product scorers must actually read (anti blackhole).
#[derive(Debug, Clone, Default)]
pub struct ScorerConsumptionSsot {
    pub version: String,
    pub required_tiers: Vec<String>,
    pub required_roles: Vec<String>,
    /// field -> reason (forgeability/physics only; never fixture-fit)
    pub intentional_unconsumed: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FieldProductMatrix {
    pub version: String,
    pub fields: Vec<FieldAxisRoles>,
    pub by_field: HashMap<String, FieldAxisRoles>,
    /// Commercial device_id material whitelist (F-11 SSOT).
    pub commercial_device_id: CommercialDeviceIdSsot,
    /// T0–T1 material/conf/veto must be referenced in scorers unless allowlisted.
    pub scorer_consumption: ScorerConsumptionSsot,
}

#[derive(Debug, Clone)]
pub struct TaskGapDef {
    pub code: String,
    pub task_ids: Vec<String>,
    pub severity: String,
    pub detail: String,
    pub candidate_packs: Vec<String>,
    pub legacy_pack_gaps: Vec<String>,
    pub requires_soft_v2: bool,
}

#[derive(Debug, Clone)]
pub struct TaskGapMap {
    pub version: String,
    pub gaps: Vec<TaskGapDef>,
    pub by_code: HashMap<String, TaskGapDef>,
    /// legacy gap code -> task gap codes that list it
    pub legacy_to_task: HashMap<String, Vec<String>>,
}

fn load_json(path: &Path) -> Result<Value, ContractError> {
    let text = fs::read_to_string(path)
        .map_err(|e| ContractError::new(format!("missing {}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| ContractError::new(format!("invalid json {}: {e}", path.display())))
}

impl FieldProductMatrix {
    pub fn load_from(spec_dir: &Path) -> Result<Self, ContractError> {
        let raw = load_json(&spec_dir.join("field_product_matrix.json"))?;
        Self::from_value(&raw)
    }

    pub fn from_value(raw: &Value) -> Result<Self, ContractError> {
        let version = raw
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let arr = raw
            .get("fields")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ContractError::new("field_product_matrix.fields must be array"))?;
        let mut fields = Vec::new();
        let mut by_field = HashMap::new();
        for (i, f) in arr.iter().enumerate() {
            let name = f
                .get("field")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ContractError::new(format!("fields[{i}].field required")))?
                .to_string();
            let primary_packs: Vec<String> = f
                .get("primary_packs")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            let mut axes = HashMap::new();
            if let Some(obj) = f.get("axes").and_then(|v| v.as_object()) {
                for (k, v) in obj {
                    if let Some(role) = v.as_str() {
                        axes.insert(k.clone(), role.to_string());
                    }
                }
            }
            let trust_tier = f
                .get("trust_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("T2")
                .to_string();
            let row = FieldAxisRoles {
                field: name.clone(),
                primary_packs,
                axes,
                trust_tier,
            };
            by_field.insert(name, row.clone());
            fields.push(row);
        }
        let commercial_device_id = parse_commercial_ssot(raw.get("commercial_device_id"));
        let scorer_consumption = parse_scorer_consumption(raw.get("scorer_consumption"));
        Ok(Self {
            version,
            fields,
            by_field,
            commercial_device_id,
            scorer_consumption,
        })
    }

    /// Fields that must appear as string literals in gr-core scorers (iss/25 contract).
    ///
    /// Returns `(field, trust_tier, roles_joined)` for T0–T1 material|conf|veto
    /// not listed in `intentional_unconsumed`.
    pub fn required_scorer_fields(&self) -> Vec<(String, String, String)> {
        let tiers = if self.scorer_consumption.required_tiers.is_empty() {
            vec!["T0".into(), "T1".into()]
        } else {
            self.scorer_consumption.required_tiers.clone()
        };
        let roles = if self.scorer_consumption.required_roles.is_empty() {
            vec!["material".into(), "veto".into(), "conf".into()]
        } else {
            self.scorer_consumption.required_roles.clone()
        };
        let mut out = Vec::new();
        for row in &self.fields {
            if !tiers.iter().any(|t| t == &row.trust_tier) {
                continue;
            }
            if self
                .scorer_consumption
                .intentional_unconsumed
                .contains_key(&row.field)
            {
                continue;
            }
            let role_list: Vec<String> = row
                .axes
                .iter()
                .filter(|(_, role)| roles.iter().any(|r| r == *role))
                .map(|(ax, role)| format!("{ax}:{role}"))
                .collect();
            if role_list.is_empty() {
                continue;
            }
            out.push((row.field.clone(), row.trust_tier.clone(), role_list.join(",")));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Fraction of material/conf/veto fields present for a product axis (0..1).
    ///
    /// Only **T0–T2** rows count — T3+ diag/conf expansion must not dilute coverage
    /// as the matrix grows (iss/18 F-1/F-11).
    pub fn axis_coverage(&self, axis: &str, fields: &Value) -> f64 {
        let fo = match fields.as_object() {
            Some(o) => o,
            None => return 0.0,
        };
        let mut need = 0u32;
        let mut have = 0u32;
        for row in &self.fields {
            let role = match row.axes.get(axis) {
                Some(r) if r == "material" || r == "conf" || r == "veto" => r.as_str(),
                _ => continue,
            };
            // material T0–T2; conf/veto only T0–T1 (avoid conf-T2 dilution)
            let tier = row.trust_tier.as_str();
            let ok = match (role, tier) {
                ("material", "T0" | "T1" | "T2") => true,
                ("conf" | "veto", "T0" | "T1") => true,
                _ => false,
            };
            if !ok {
                continue;
            }
            // Nested automation.* fields
            need += 1;
            let present = if row.field.contains('.') {
                let parts: Vec<&str> = row.field.splitn(2, '.').collect();
                fo.get(parts[0])
                    .and_then(|v| v.get(parts[1]))
                    .map(|v| !v.is_null() && v.as_str() != Some(""))
                    .unwrap_or(false)
                    || fo
                        .get(&row.field)
                        .map(|v| !v.is_null())
                        .unwrap_or(false)
            } else {
                fo.get(&row.field)
                    .map(|v| {
                        if v.is_null() {
                            false
                        } else if let Some(s) = v.as_str() {
                            !s.is_empty()
                        } else if let Some(a) = v.as_array() {
                            !a.is_empty()
                        } else {
                            true
                        }
                    })
                    .unwrap_or(false)
            };
            if present {
                have += 1;
            }
            // veto materials count even if false bool (webdriver:false still present)
            let _ = role;
        }
        if need == 0 {
            return 0.0;
        }
        have as f64 / need as f64
    }

    /// Fields that contribute material role to multiple product axes (shared evidence).
    pub fn multi_axis_material_fields(&self) -> Vec<String> {
        self.fields
            .iter()
            .filter(|r| {
                r.axes
                    .values()
                    .filter(|role| *role == "material" || *role == "conf")
                    .count()
                    >= 2
            })
            .map(|r| r.field.clone())
            .collect()
    }
}

impl TaskGapMap {
    pub fn load_from(spec_dir: &Path) -> Result<Self, ContractError> {
        let raw = load_json(&spec_dir.join("task_gap_map.json"))?;
        Self::from_value(&raw)
    }

    pub fn from_value(raw: &Value) -> Result<Self, ContractError> {
        let version = raw
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let arr = raw
            .get("gaps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ContractError::new("task_gap_map.gaps must be array"))?;
        let mut gaps = Vec::new();
        let mut by_code = HashMap::new();
        let mut legacy_to_task: HashMap<String, Vec<String>> = HashMap::new();
        for (i, g) in arr.iter().enumerate() {
            let code = g
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ContractError::new(format!("gaps[{i}].code required")))?
                .to_string();
            let task_ids: Vec<String> = g
                .get("task_ids")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            let severity = g
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("mid")
                .to_string();
            let detail = g
                .get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let candidate_packs: Vec<String> = g
                .get("candidate_packs")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            let legacy_pack_gaps: Vec<String> = g
                .get("legacy_pack_gaps")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            let requires_soft_v2 = g
                .get("requires_soft_v2")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            for leg in &legacy_pack_gaps {
                legacy_to_task
                    .entry(leg.clone())
                    .or_default()
                    .push(code.clone());
            }
            let def = TaskGapDef {
                code: code.clone(),
                task_ids,
                severity,
                detail,
                candidate_packs,
                legacy_pack_gaps,
                requires_soft_v2,
            };
            by_code.insert(code, def.clone());
            gaps.push(def);
        }
        Ok(Self {
            version,
            gaps,
            by_code,
            legacy_to_task,
        })
    }

    pub fn packs_for_gap_codes<'a>(
        &'a self,
        codes: impl IntoIterator<Item = &'a str>,
        soft_v2_ready: bool,
    ) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut codes: Vec<&str> = codes.into_iter().collect();
        codes.sort();
        for c in codes {
            // direct task code
            if let Some(def) = self.by_code.get(c) {
                if def.requires_soft_v2 && !soft_v2_ready {
                    continue;
                }
                for p in &def.candidate_packs {
                    if seen.insert(p.as_str()) {
                        out.push(p.as_str());
                    }
                }
            }
            // legacy → task
            if let Some(task_codes) = self.legacy_to_task.get(c) {
                let mut tcs: Vec<&str> = task_codes.iter().map(|s| s.as_str()).collect();
                tcs.sort();
                for tc in tcs {
                    if let Some(def) = self.by_code.get(tc) {
                        if def.requires_soft_v2 && !soft_v2_ready {
                            continue;
                        }
                        for p in &def.candidate_packs {
                            if seen.insert(p.as_str()) {
                                out.push(p.as_str());
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

fn parse_str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn parse_commercial_ssot(raw: Option<&Value>) -> CommercialDeviceIdSsot {
    let Some(obj) = raw.and_then(|v| v.as_object()) else {
        return CommercialDeviceIdSsot::default();
    };
    CommercialDeviceIdSsot {
        digest_order: parse_str_list(obj.get("digest_order")),
        hardware_anchors: parse_str_list(obj.get("hardware_anchors")),
        soft_commercial_extra: parse_str_list(obj.get("soft_commercial_extra")),
        never_digest: parse_str_list(obj.get("never_digest")),
    }
}

fn parse_scorer_consumption(raw: Option<&Value>) -> ScorerConsumptionSsot {
    let Some(obj) = raw.and_then(|v| v.as_object()) else {
        // Default policy matches iss/25 even if JSON section missing (fail-closed on material).
        return ScorerConsumptionSsot {
            version: "default".into(),
            required_tiers: vec!["T0".into(), "T1".into()],
            required_roles: vec!["material".into(), "veto".into(), "conf".into()],
            intentional_unconsumed: HashMap::new(),
        };
    };
    let mut intentional = HashMap::new();
    if let Some(m) = obj.get("intentional_unconsumed").and_then(|v| v.as_object()) {
        for (k, v) in m {
            if let Some(reason) = v.as_str() {
                intentional.insert(k.clone(), reason.to_string());
            }
        }
    }
    ScorerConsumptionSsot {
        version: obj
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1")
            .to_string(),
        required_tiers: {
            let t = parse_str_list(obj.get("required_tiers"));
            if t.is_empty() {
                vec!["T0".into(), "T1".into()]
            } else {
                t
            }
        },
        required_roles: {
            let r = parse_str_list(obj.get("required_roles"));
            if r.is_empty() {
                vec!["material".into(), "veto".into(), "conf".into()]
            } else {
                r
            }
        },
        intentional_unconsumed: intentional,
    }
}

static FIELD_MATRIX: OnceLock<Result<FieldProductMatrix, String>> = OnceLock::new();
static TASK_GAPS: OnceLock<Result<TaskGapMap, String>> = OnceLock::new();

pub fn load_field_product_matrix() -> Result<&'static FieldProductMatrix, ContractError> {
    let cached = FIELD_MATRIX.get_or_init(|| {
        FieldProductMatrix::load_from(&find_spec_dir()).map_err(|e| e.to_string())
    });
    match cached {
        Ok(m) => Ok(m),
        Err(e) => Err(ContractError::new(e.clone())),
    }
}

/// Commercial digest key order from matrix SSOT, with built-in fallback.
pub fn commercial_digest_order() -> Vec<&'static str> {
    if let Ok(m) = load_field_product_matrix() {
        if !m.commercial_device_id.digest_order.is_empty() {
            // Leak strings into process for 'static lifetime of slice refs used by trust.
            // Prefer holding owned via thread-local would be complex; use static fallback
            // when SSOT matches known order, else leak once.
            static ORDER: OnceLock<Vec<String>> = OnceLock::new();
            let owned = ORDER.get_or_init(|| m.commercial_device_id.digest_order.clone());
            return owned.iter().map(|s| s.as_str()).collect();
        }
    }
    vec![
        "form_class",
        "hw_audio_stable",
        "hw_webgl_stable",
        "cores_class",
        "architecture",
    ]
}

pub fn commercial_hardware_anchors() -> Vec<&'static str> {
    if let Ok(m) = load_field_product_matrix() {
        if !m.commercial_device_id.hardware_anchors.is_empty() {
            static A: OnceLock<Vec<String>> = OnceLock::new();
            let owned = A.get_or_init(|| m.commercial_device_id.hardware_anchors.clone());
            return owned.iter().map(|s| s.as_str()).collect();
        }
    }
    vec!["hw_audio_stable", "hw_webgl_stable"]
}

pub fn commercial_soft_extra() -> Vec<&'static str> {
    if let Ok(m) = load_field_product_matrix() {
        if !m.commercial_device_id.soft_commercial_extra.is_empty() {
            static E: OnceLock<Vec<String>> = OnceLock::new();
            let owned = E.get_or_init(|| m.commercial_device_id.soft_commercial_extra.clone());
            return owned.iter().map(|s| s.as_str()).collect();
        }
    }
    vec![
        "cores_class",
        "architecture",
        "os_family",
        "webrtc_host_ip_hash",
    ]
}

pub fn load_task_gap_map() -> Result<&'static TaskGapMap, ContractError> {
    let cached = TASK_GAPS
        .get_or_init(|| TaskGapMap::load_from(&find_spec_dir()).map_err(|e| e.to_string()));
    match cached {
        Ok(m) => Ok(m),
        Err(e) => Err(ContractError::new(e.clone())),
    }
}

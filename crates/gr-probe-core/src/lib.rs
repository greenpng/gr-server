#![recursion_limit = "512"]
//! green-v5 core library — independent Rust hot path.
//!
//! No path deps on greenv4 siblings (feasibility-*, probe/crates).
//! Specs load from the tree-local `spec/` directory (or CARGO_MANIFEST_DIR embed).

/// Product release tag — **SSOT** from tree-root `VERSION` (build.rs → rustc-env).
/// Same string is used for analyze JSON, cool gating, FE inject `?v=`, and SDK boot.version.
pub const GR_PRODUCT_VERSION: &str = env!("GR_PRODUCT_VERSION");
/// P0-1: per-release random build id (release script sets GR_BUILD_ID; dev = "dev").
/// Same id is written into the release manifest, so every build differs and a
/// patch derived from a previous binary cannot be reused.
pub const GR_BUILD_ID: &str = env!("GR_BUILD_ID");
/// FE/SDK cache-bust tag — **unified** with product (do not diverge).
pub const GR_FE_VERSION: &str = env!("GR_PRODUCT_VERSION");

pub mod algo_groups;
pub mod analysis_quality;
pub mod association_ladder;
pub mod brain_hypotheses;
pub mod challenge_pow;
pub mod challenge_rotate;
pub mod collision_kpi;
pub mod composite_association;
pub mod conf_cal;
pub mod contracts;
pub mod corroborate;
pub mod curve_morph;
pub mod gateway_coherence;
pub mod puf_metrics;
pub mod ua_ch_headers;
pub mod bot;
pub mod bot_weights;
pub mod brain;
pub mod brain_control;
pub mod brain_directions;
pub mod capabilities;
pub mod catalog;
pub mod client_exec;
pub mod client_ip;
pub mod net_enrich;
pub mod license_token;
pub mod panel_policy;
pub mod plan_entitlement;
pub mod privacy;
pub mod strategy_vault;
pub mod decision;
pub mod seal;
pub mod seal_v2;
pub mod device_ensemble;
pub mod device_tier;
pub mod device_segments;
pub mod analyze_schedule;
pub mod analyze_mask;
pub mod probe_field_priority;
pub mod association_api;
pub mod peer_similarity;
pub mod edh;
pub mod engine_surface;
pub mod evidence_ledger;
pub mod probe_domain;
pub mod utilization;
pub mod stack_cooccurrence;
pub mod evaluate;
pub mod experiment;
pub mod fp_channel_scores;
pub mod hub_promotion;
pub mod inject;
pub mod ja_population_prior;
pub mod link;
pub mod link_or_mint;
pub mod neg_dict;
pub mod policy;
pub mod product_matrix;
pub mod product_scores;
pub mod protocol_edge;
pub mod r100_templates;
pub mod return_gate;
pub mod sdk_projection;
pub mod selection_provenance;
pub mod product_public;
pub mod observation_envelope;
pub mod response_slim;
pub mod result_projection;
pub mod business_context;
pub mod config_snapshot;
pub mod entropy_census;
pub mod cluster_lsh;
pub mod rpa_bands;
pub mod behavior_self_sim;
pub mod claim_obs_graph;
pub mod ja4h_lite;
pub mod hw_anti_collision;
pub mod hw_silicon_fusion;
pub mod hw_channel_census;
pub mod hw_channel_drift;
pub mod hw_fusion_weights;
pub mod hw_engine_norm;
pub mod identity_governance;
pub mod hw_clock_skew;
pub mod population_atlas;
pub mod model_key;
pub mod atlas_score;
pub use model_key::{
    attach_model_key_extras, commercial_body_excludes_plaintext_model, commercial_model_key_ready,
    model_key_from_fields, parse_unmasked_renderer, MODEL_KEY_ALGO,
};
pub use atlas_score::{atlas_shadow_score, ATLAS_SCORE_ALGO};
pub mod ops_taxonomy;
pub use ops_taxonomy::{enrich_ops_event, OPS_TAXONOMY_ALGO};
pub mod shared_governance;
pub mod shared_l2;
pub mod hnsw_lite;
pub mod contrastive_sup;
pub mod fuzzy_ecc;
pub mod hw_probe_analysis;
pub mod physical_assert;
pub mod antidetect_signals;
pub mod rule_sample_loader;
pub mod self_capability;
pub mod session_ticket;
pub mod storage_bind;
pub mod sla;
pub mod soft_v2;
pub mod stack_auth;
pub mod tls_ja4;
pub mod trust;
pub mod unknown_hub;
pub mod xsrc;

pub use analysis_quality::{
    attach_analysis_quality, axis_confidence_report, build_field_utilization_map,
    build_material_participation, build_mutual_verification, classify_open_gaps,
    classify_platform_honesty,
    open_verification_gaps, task_critical_fields,
};
pub use brain_hypotheses::{
    activate_hypotheses, elevated_packs_for_fields, hypothesis_coverage_json, packs_for_hypothesis,
    H_AD, H_CDP, H_EMU, H_REAL, H_VM,
};
pub use product_scores::{
    build_ops_fusion_telemetry, build_product_surface, build_product_surface_with_evidence,
    collect_reference_aux_hits, field_density, os_br_completeness, score_br, score_os, score_rpa,
};
pub use product_public::{
    list_strategy_presets, project as project_public, project_with as project_public_with,
    signal_dictionary, ProjectCtx, ResponseProfile, StrategyId, StrategyPreset, PRODUCT_PUBLIC_ALGO,
};
pub use observation_envelope::{
    extract_cf_edge_fields, infer_realm_kind, infer_source_kind, merge_cf_into_gateway_fields,
    signal_meta, stamp_observation_envelope, ClaimedEnvelope, OBSERVATION_ENVELOPE_SCHEMA,
};
pub use result_projection::{
    assemble_result_response, default_result_projection, diagnostic_scope_from_headers,
    resolve_result_projection, result_token_enforced, ResultProjection,
};
pub use business_context::{
    canonicalize_business_context, project_business_context, BUSINESS_CONTEXT_SCHEMA,
};
pub use rule_sample_loader::{
    apply_rule_samples_axis, evaluate_rule_samples_shadow, load_rule_sample_pack,
};
pub use self_capability::{load_self_capability, self_capability_json};
pub use client_ip::{client_ip_from, client_ip_source, peer_is_trusted_proxy};
pub use net_enrich::{
    classify_ip, enrich_fields_if_empty, ja4_ua_mismatch, ja4_ua_mismatch_ex,
};
pub use panel_policy::{
    load_cached as load_panel_policy, merge_entitlements, merge_result_policy, merge_retention, merge_strategy,
    policy_path as panel_policy_path, policy_public_view, save as save_panel_policy, PanelPolicy,
    ResultPolicy, RetentionPolicy, StrategyPolicy,
};
pub use seal::{
    derive_session_seal_secret, issue_session_seal_grant, seal_probe_payload,
    seal_probe_payload_ex, seal_probe_payload_ex2, unseal_probe_payload,
    unseal_probe_payload_auto, unseal_probe_payload_auto_legacy, unseal_probe_payload_rotated,
    decompress_limited, SealedEnvelope, SealError, MAX_DECOMPRESSED_BYTES, SESSION_SEAL_TTL_MS,
};
pub use seal_v2::{
    allowed_fe_epochs, compute_challenge_bind, compute_pack_set_hash, current_suite_id,
    current_wasm_module_id, derive_session_seal_secret_v2, issue_session_seal_grant_v2,
    seal_probe_payload_v2, seal_require_v2, seal_v2_public_meta, suite_allowed,
    validate_envelope_v2_policy, validate_unsealed_content, wasm_module_allowed,
    B10_ALLOWED_ALGO_PREFIXES, SEAL_SUITE_S2_AESGCM_HKDF_V1, SEAL_WASM_ASSET, SEAL_WASM_MODULE_ID,
};
pub use hub_promotion::hub_promotion_drafts;
pub use ja_population_prior::{
    apply_ja_population_prior_br, ja_population_prior_json, load_ja_population_prior,
};
pub use stack_cooccurrence::{
    extract_stack_dims, load_stack_cooccurrence_prior, lookup_stack_cooccurrence,
    stack_cooccurrence_json,
};
pub use challenge_rotate::{next_rotation, rotate_challenge_epoch};
pub use association_ladder::{association_ladder, ASSOCIATION_LADDER_ALGO};
pub use peer_similarity::{
    compute_peer_similarity, environment_flags, pair_peer_contrib, sanitize_custom_link_fields,
    DEFAULT_WINDOW_SEC, PEER_SIMILARITY_ALGO,
};
pub use composite_association::{
    compare_assoc_features, composite_associate, extract_assoc_features, self_association_readiness,
    AssocFeatures, COMPOSITE_ASSOCIATION_ALGO,
};
pub use challenge_pow::{
    evaluate_challenge_fields, issue_challenge_seed, validate_challenge_seed, CHALLENGE_ALGO,
    DEFAULT_CHALLENGE_SECRET, DEFAULT_CHALLENGE_TTL_MS,
};
pub use neg_dict::{derive_neg_dict_hits, neg_dict_spoof_boost, NEG_DICT_CODES};
pub use r100_templates::{
    catalog_summary, is_r100_pack_id, redis_seed_entries, render_register_pack_js,
    R100TemplateCatalog, R100_REDIS_PREFIX, R100_REDIS_SET, R100_TEMPLATES_ALGO,
};
pub use protocol_edge::{
    annotate_protocol_export_honesty, brand_to_engine_family, classify_protocol_engine,
    derive_engine_claim_obs, headers_map_from_pairs, inject_protocol_from_headers,
    PROTOCOL_DEPTH_EXPORT_KEYS, PROTOCOL_DIAGNOSTIC_ONLY_KEYS, TRUSTED_H2_HEADERS,
    TRUSTED_JA4_HEADERS,
};
pub use tls_ja4::{
    compute_fingerprints, compute_ja4t_from_option_kinds, fingerprint_client_hello,
    inject_tls_fp_fields, is_grease_u16, parse_client_hello, ClientHelloParts, TlsFingerprint,
};
pub use ua_ch_headers::{
    inject_gateway_ua_ch, normalize_sec_ch_ua_brands, sec_ch_full_version_list_v,
    TRUSTED_SEC_CH_HEADERS,
};
pub use corroborate::{
    count_material_families, fuse_axis_hedge, fuse_channels, material_families_for_axis,
    material_vote_channel, multi_source_channel, ChannelOut, Stance,
};
pub use bot::{has_strong_bot, robot_name_from_ua, score_bot, BotScore};
pub use brain::{
    build_frontier, coverage_checklist, filter_client_proposed_packs, filter_unresolved_packs,
    scan_gaps, static_kick_plan, FrontierPlan, Gap,
};
pub use brain_control::{
    apply_band_pack_budget, apply_tiered_pack_budget, pack_lane, PackLane, belief_from_fields_thin,
    build_battle_log, build_belief,
    build_hw_safe_parallel_groups, classify_stop_reason, is_dense_pack_id,
    is_hardware_probe_pack, is_verify_rand_pack, max_packs_for_band, policy_band_from_scores,
    resource_class, shuffle_packs_for_source,
    scan_capability_envelope, verify_spotcheck_n_for_session, DENSE_PACKS_PER_TICK_CAP,
    VERIFY_SPOTCHECK_MAX, VERIFY_SPOTCHECK_MIN, VERIFY_SPOTCHECK_PER_TICK, select_missions,
    sign_route_plan, unknown_bucket_from_belief, update_direction_priors,
};
pub use brain_directions::{
    mission_allowed_directions, rank_directions_for_missions, schedule_packs_from_directions_missions,
};
pub use brain_directions::{
    rank_directions, sandbox_stage_plan, schedule_packs_from_directions, DIRECTIONS,
};
pub use capabilities::{filter_packs_by_capabilities, project_capabilities};
pub use catalog::{load_catalog, ComponentCatalog, PackDef};
pub use client_exec::{
    apply_client_exec_demotion, assess_client_execution, client_exec_forces_unknown,
    client_exec_is_thin,
};
pub use contracts::{
    coverage_for_batches, load_all_specs, present_packages, validate_route_plan, ContractError,
    Specs,
};
pub use decision::{
    decide_from_evaluate_parts, decide_from_evaluate_parts_scored, decide_product_action,
    derive_recommended_action, ActionSensitivity, DecisionInput, DecisionOutcome,
    CONFIDENCE_VERSION,
};
pub use policy::{
    load_product_policy, multi_source_conflict_weights, otp_thresholds, MultiSourceConflictWeights,
    DEFAULT_POLICY_ID,
};
pub use evaluate::{evaluate_session, multi_session_ensemble, multi_session_link};
pub use edh::{
    b10x_fields_terminal, b10x_silicon_required_packs, build_edh, count_b10x_batches,
    evidence_has_b10x, missing_b10x_silicon, silicon_pack_attempt_complete,
};
pub use product_matrix::{
    load_field_product_matrix, load_task_gap_map, FieldProductMatrix, TaskGapMap,
};
pub use device_ensemble::{
    associate_all, digest_strategy_votes, fuse_pair_ensemble, fuse_single_ensemble, ENSEMBLE_ALGO,
};
pub mod multi_source_mint;
pub use multi_source_mint::{
    assess_mint_gate, conflict_score_demotion, field_mint_decision, host_sep_allowed_for_mint,
    residual_allowed_for_mint, resolve_fields_multi_source, MINT_SEMANTIC_KEYS,
};
pub mod source_trust;
pub use source_trust::{
    build_source_auth_ladder_view, key_class, rank_sources_for_key, resolve_field_source,
    source_can_supply, source_trust_for, KeyClass, SourceTrust,
};
pub use algo_groups::{
    b10x_schedule_hint, commercial_id_from_selection, field_registry_json, same_commercial_bucket,
    score_materials_boost, select_identity_group, select_vt_best_silicon, ALGO_GROUPS_ALGO,
    FIELD_REGISTRY_ALGO,
};
pub use device_tier::{
    authentic_fields_for_device_id, dimension_sufficiency, qualifies_dh, qualifies_dh_with_evidence,
    qualifies_dv, qualifies_dv_with_evidence, select_device_tier, DimSufficiency, DEVICE_TIER_ALGO,
    commercial_family, device_id_body, format_algo_device_id, is_commercial_device_id,
    is_dg_id, is_dh_id, is_dv_id, parse_algo_group, DEVICE_ALGO_GROUPS,
};
pub use device_segments::{
    clear_device_segments_provider, curve_descriptors_for_sdk, curve_lsh_public,
    device_segment_composition_json, device_segments_provider_active,
    install_device_segments_provider, is_multi_segment_id, select_device_segments,
    select_device_segments_local, DEVICE_SEGMENTS_ALGO, SEGMENT_PART_ORDER, SEGMENT_PREFIXES,
};
pub use analyze_schedule::{
    analyze_arm_debounce_ms_after_ingest, analyze_arm_due_ms_after_ingest, analyze_arm_due_ms_pre_cold,
    analyze_due_now, analyze_schedule_policy_json, new_batch_analyze_clocks, set_analyze_idle_upload_ms,
    AnalyzeDueReason, ANALYZE_IDLE_UPLOAD_MS, ANALYZE_NO_RESULT_MS, ANALYZE_SCHEDULE_ALGO,
};
pub use probe_field_priority::probe_field_priority_json;
pub use association_api::{
    assess_from_events, reject_raw_pii_fields, subject_ref_from_secret, validate_subject_ref,
    ASSOC_SCHEMA_VERSION, SUBJECT_REF_PREFIX,
};
pub use return_gate::{
    apply_identity_return_gate, complete_on_commercial_silicon, sdk_slim_projection,
    set_complete_on_commercial_silicon, should_analyze_page_rpa, should_return_identity_to_sdk,
    should_return_identity_to_sdk_ex, RPA_IDLE_ANALYZE_MS, SDK_RETURN_IDLE_MS, sdk_return_idle_ms,
    set_sdk_return_idle_ms,
};
pub use sdk_projection::SDK_PROJECTION_ALGO;
pub use config_snapshot::{current_config_snapshot, push_config_overlay, CONFIG_SNAPSHOT_ALGO};
pub use entropy_census::{
    census_from_curve_descriptor_batch, census_slot_digests, ENTROPY_CENSUS_ALGO,
};
pub use cluster_lsh::{cluster_by_curve_lsh, hex_digest_distance, CLUSTER_LSH_ALGO};
pub use identity_governance::{
    apply_identity_governance, catalog_register, inject_deepen_packs, machine_heat_unit,
    mu_census_snapshot, mu_opts_from_governance, observe_bucket_heat_ex, observe_mu_census,
    observe_mu_census_ex, pair_match_score, resolve_identity_candidates, slot_effective_bits,
    IDENTITY_GOVERNANCE_ALGO, MergePosture, MuObserveOpts,
};
pub use hw_clock_skew::{estimate_clock_skew, estimate_from_fields as estimate_clock_skew_from_fields, HW_CLOCK_SKEW_ALGO};
pub use population_atlas::{
    atlas_observe, cohort_key_from_fields, differential_encode, POPULATION_ATLAS_ALGO,
};
pub use shared_governance::{
    clear_shared_governance_files, set_shared_governance_dir_for_tests, shared_governance_metrics,
    shared_state_paths, test_isolation_dir,
};
pub use shared_l2::{l2_enabled, l2_metrics, SHARED_L2_ALGO};
pub use hnsw_lite::{
    contrastive_embed, contrastive_embed_base, embed_base_from_fields, hnsw_insert, hnsw_search,
    hnsw_stats, HNSW_LITE_ALGO,
};
pub use contrastive_sup::{
    contrastive_stats, embed_supervised_from_fields, fit_contrastive_from_pairs,
    fit_contrastive_synthetic_fleet, observe_contrastive_online, pair_cosine_supervised,
    CONTRASTIVE_SUP_ALGO,
};
pub use fuzzy_ecc::{fuzzy_ecc_from_fields, fuzzy_same_machine_rate, FUZZY_ECC_ALGO};
pub use hw_probe_analysis::{
    analyze_timing_like, multiround_median, scale_invariant_mean, HW_PROBE_ANALYSIS_ALGO,
};
pub use physical_assert::{
    physical_assert_surface, physical_contradiction, physical_plausibility, PHYSICAL_CONTRADICTION_ALGO,
    PHYSICAL_PLAUSIBILITY_ALGO,
};
pub use antidetect_signals::{
    antidetect_signals_surface, behavior_session_signals, ja4_full_surface, profile_farm_report,
    ANTIDETECT_BUNDLE_ALGO, BEHAVIOR_SESSION_ALGO, JA4_FULL_ALGO,
};
pub use rpa_bands::{rpa_behavior_bands, RPA_BANDS_ALGO};
pub use behavior_self_sim::{
    behavior_profile_vec32, cosine_vec32, offline_ece_from_spec_or_json, BEHAVIOR_SELF_SIM_ALGO,
    BEHAVIOR_VEC32_DIM,
};
pub use claim_obs_graph::{
    build_claim_obs_graph, canary_model_compare, claim_obs_and_shadow, claim_obs_shadow_canary,
    shadow_score_from_graph, CLAIM_OBS_GRAPH_ALGO, SHADOW_SCORE_ALGO,
};
pub use ja4h_lite::{ensure_ja4h_fields, ja4h_lite_product, JA4H_LITE_ALGO};
pub use hw_anti_collision::{
    anti_collision_materials, build_anti_collision_surface, complex_curve_features,
    TeachingParams, HW_ANTI_COLLISION_ALGO,
};
pub use hw_silicon_fusion::{
    fuse_silicon_channels, prefer_fused_curves, silicon_fusion_materials, HW_SILICON_FUSION_ALGO,
};
pub use hw_channel_census::{
    channel_census_materials, hw_channel_census, HW_CHANNEL_CENSUS_ALGO,
};
pub use hw_channel_drift::{
    channel_drift_from_fields, channel_drift_job, channel_drift_job_from_dir, channel_drift_report,
    drift_gate_enabled, stability_gate_from_fields, HW_CHANNEL_DRIFT_ALGO,
    HW_CHANNEL_DRIFT_JOB_ALGO,
};
pub use hw_fusion_weights::{
    adopt_fusion_weights, fit_fusion_weights_from_pairs, fusion_weights_ops, fusion_weights_status,
    load_fusion_weights, load_fusion_weights_fresh, write_fusion_weights_candidate,
    SILICON_FUSION_WEIGHTS_ALGO, SILICON_FUSION_WEIGHTS_FIT_ALGO,
};
pub use hw_engine_norm::{
    adopt_engine_norm, calibrate_engine_norm_from_batch, detect_engine_from_fields,
    engine_norm_ops, engine_norm_status, engine_norm_table_json, load_engine_norm_table,
    normalize_engine_key, normalize_lane_s_curve, write_engine_norm_candidate,
    LANE_S_ENGINE_NORM_ALGO, LANE_S_ENGINE_NORM_CAL_ALGO,
};
pub use session_ticket::{
    b10_sla_violation, evidence_has_b10, gaps_need_b10, has_silicon_materials,
    issue_session_ticket, issue_session_ticket_versioned, session_probe_sufficient,
    silicon_materials_fingerprint,
    should_skip_session_probe, validate_session_ticket, validate_session_ticket_ex,
    B10_SLA_MS, TICKET_TTL_MS,
};
pub use storage_bind::{issue_storage_bind, validate_storage_bind, sign_relay_body, verify_relay_sig, sha256_hex};

pub use experiment::{apply_strategy, default_stable_policy, StrategyApplyResult};
pub use inject::{
    inject_deploy_gate, plan_inject, simulate_document_boot, validate_inject_config, InjectError,
    InjectPlan,
};
pub use link::{
    associate, browser_surface_id_from_fields, canonical_webgl_renderer,
    commercial_device_id_from_fields, gpu_key, machine_materials, normalize_webgl_renderer,
    LinkResult,
};
pub use link_or_mint::{
    apply_server_mint, apply_server_mint_with_evidence, binder_obs_from_fields,
    field_algorithm_weights, has_host_separator, honest_digest_path, is_empty_anchor,
    is_thin_surface, link_or_mint_pair, link_threshold, score_link, score_link_fields,
    score_link_legacy, server_mint_commercial_id,
    FileDeviceIndex, MemoryDeviceIndex, DIGEST_PATH_EMPTY, DIGEST_PATH_GATEWAY,
    DIGEST_PATH_SOFT_UNIT_V1, DIGEST_PATH_THIN, DIGEST_PATH_UNIT_V1, LINK_OR_MINT_ALGO,
    LINK_THRESHOLD, SERVER_MINT_ALGO, UNIT_SURFACE_ALGO_V1,
};
pub use collision_kpi::{
    collision_posture_for_fields, report_collision_kpi, report_profile_boundary_kpi,
    COLLISION_KPI_ALGO,
};
pub use conf_cal::{
    active_confidence_version, adopt_decision_report, adopt_fs_thresholds, adopt_ladder_calibration,
    adopted_fs_taus, calibrate_fs_thresholds, calibrate_offline, calibrate_offline_by_ladder,
    confidence_adopt_requested, confidence_calibration_ref, ece_from_customer_labels,
    load_spec_labeled_pairs, pairs_from_customer_labels, pairs_from_json, pairs_from_matrix_cells,
    LabeledPair, CALIBRATED_CONFIDENCE_VERSION, RUNTIME_CONFIDENCE_VERSION,
};
pub use sla::{
    aggregate_identity_sla, evaluate_sla_alerts, rows_from_json_array, sla_alert_thresholds,
    SlaAlertThresholds, SlaSessionRow,
};
pub use soft_v2::{
    build_soft_graph, coarse_key_hamming, commercial_id_heat_report, cosine_similarity,
    curve_coarse_sig, extract_hw_curves, homogenization_product_signal, hw_curves_coarse_key,
    hw_noise_similarity, member_from_evidence, soft_link_mode, soft_pair_decision, DeviceIdHeat,
    FileSoftEdgeStore, MemorySoftEdgeStore, SoftBlockingEngine, SoftConfig, SoftEdge,
    SoftEdgeStore, SoftMember, PROMOTE_TO_COMMERCIAL_ID, SOFT_FUSE_OWNER,
    SOFT_MISRECALL_FUSE_DEFAULT,
};
pub use stack_auth::{
    commercial_id_blocked_by_stack, gpu_label_commercial_ok, infer_residual_soft_like,
    infer_residual_soft_like_mean, is_soft_renderer_class, renderer_class_from_label,
    stack_auth_from_fields, StackAuth, STACK_AUTH_ALGO,
};
pub use trust::{
    commercial_device_id_trusted, commercial_projection, cores_class, curve_stable_digest,
    engine_family_from_fields, residual_curve_for_engine, residual_magrank_key,
    select_residual_curve_from_paths, curve_stable_digest_coarse, eligibility_reason_codes,
    material_trust_prior, materials_detail, residual_curve_entropy_ok, trust_materials,
    multipath_commercial_fuse, webgl_commercial_digest, webgl_commercial_digest_for_engine,
    webgl_peak_signature,
    COMMERCIAL_ALGO, COMMERCIAL_TRUST_FLOOR, COMMERCIAL_TRUST_SUM_MIN,
};
pub use unknown_hub::{aggregate_unknown_buckets, aggregate_unknown_from_meta_map};
pub use xsrc::{evaluate_xsrc, TruthResult};

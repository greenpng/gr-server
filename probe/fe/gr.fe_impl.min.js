/**
 * FE implementation identity — distinct from server product_version.
 * Build injects window.__GR_BUILD_IMPL__ (= root VERSION) into each ship artifact.
 * Runtime records which modules actually loaded so every batch can stamp fe_impl_*.
 *
 * Performance: O(1) memory writes; no network; upload adds a few short strings only.
 */
(function (global) {
  "use strict";
  function buildImpl() {
    try {
      if (global.__GR_BUILD_IMPL__) return String(global.__GR_BUILD_IMPL__);
    } catch (e0) {}
    return "";
  }
  function ensure() {
    try {
      if (!global.__GR_FE_IMPL__ || typeof global.__GR_FE_IMPL__ !== "object") {
        global.__GR_FE_IMPL__ = Object.create(null);
      }
      return global.__GR_FE_IMPL__;
    } catch (e1) {
      return Object.create(null);
    }
  }
  /** Record that module `name` executed from build `ver` (default: this file's build). */
  function noteModule(name, ver) {
    try {
      var m = ensure();
      var v = String(ver || buildImpl() || "");
      if (name) m[String(name)] = v;
      if (v) {
        if (!m.build) m.build = v;
        // Prefer strongest content-proven modules for "fe_impl_version"
        if (name === "lite" || name === "hard" || name === "loader" || name === "entry") {
          if (name === "lite" || name === "hard") m.content = v;
        }
      }
      if (v && !global.__GR_FE_IMPL_VERSION__) {
        global.__GR_FE_IMPL_VERSION__ = v;
      }
      if (name === "lite" && v) {
        global.__GR_FE_LITE_IMPL__ = v;
      }
      if (name === "hard" && v) {
        global.__GR_FE_HARD_IMPL__ = v;
      }
    } catch (e2) {}
  }
  /** Server product epoch (grant / inject) — must match seal allowlist. */
  function serverProduct() {
    try {
      if (global.__GR_SERVER_PRODUCT_VERSION__)
        return String(global.__GR_SERVER_PRODUCT_VERSION__);
    } catch (eS0) {}
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (eS1) {}
    try {
      var b = global.__GR_BOOT__ || {};
      if (b.product_version) return String(b.product_version);
      if (b.version) return String(b.version);
    } catch (eS2) {}
    return "";
  }

  /** Snapshot for upload stamping (shallow). */
  function snapshot() {
    var m = ensure();
    var build = buildImpl() || m.build || "";
    var content = m.content || m.lite || m.hard || "";
    var loader = m.loader || "";
    var entry = m.entry || "";
    var product = serverProduct();
    // Seal gate binds fe_impl_version to product epoch. Prefer server product so
    // sticky/old module BUILD_IMPL stamps (v5.8.*) never reject B10 in lab/prod.
    // Content/build remain diagnostic side fields.
    var effective = product || content || build || "";
    try {
      if (effective) global.__GR_FE_IMPL_VERSION__ = effective;
    } catch (eEff) {}
    return {
      build: build,
      loader: loader,
      entry: entry,
      lite: m.lite || "",
      hard: m.hard || "",
      content: content,
      product: product,
      fe_impl_version: effective,
      fe_loader_impl: loader || build,
      fe_entry_impl: entry || "",
      fe_lite_impl: m.lite || "",
      fe_hard_impl: m.hard || "",
    };
  }
  function markContentProven(ver) {
    try {
      var v = String(ver || buildImpl() || "");
      var m = ensure();
      if (v) {
        m.content = v;
        m.lite = m.lite || v;
        m.hard = m.hard || v;
        global.__GR_FE_IMPL_VERSION__ = v;
        global.__GR_FE_PACKS_VERSION__ = v;
        global.__GR_FE_CODE_VERSION__ = v;
      }
    } catch (e3) {}
  }
  global.GRFeImpl = {
    buildImpl: buildImpl,
    noteModule: noteModule,
    snapshot: snapshot,
    markContentProven: markContentProven,
    ensure: ensure,
  };
  // Self-register if this file carries a build stamp.
  try {
    if (buildImpl()) noteModule("fe_impl_lib", buildImpl());
  } catch (e4) {}
})(typeof window !== "undefined" ? window : globalThis);

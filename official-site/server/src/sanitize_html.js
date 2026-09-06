/**
 * Strict HTML sanitizer for CMS blocks (GV6-WEB-001 / GV6-WEB-022).
 * Allowlist tags/attrs; http(s)/mailto/relative URLs only; no event handlers,
 * javascript:, data:, vbscript:, protocol-relative, or control characters.
 */

const ALLOWED_TAGS = new Set([
  "p",
  "br",
  "strong",
  "b",
  "em",
  "i",
  "u",
  "s",
  "h2",
  "h3",
  "h4",
  "ul",
  "ol",
  "li",
  "blockquote",
  "code",
  "pre",
  "a",
  "span",
  "div",
  "hr",
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
]);

const ALLOWED_ATTRS = new Set(["href", "title", "class", "colspan", "rowspan"]);

function escapeText(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function hasCtl(s) {
  return /[\u0000-\u001f\u007f\\]/.test(s);
}

function decodedCompact(raw) {
  let d = String(raw || "");
  try {
    d = decodeURIComponent(d.replace(/\+/g, "%20"));
  } catch {
    /* keep raw */
  }
  return d.replace(/[\s\u0000-\u001f\u007f]/g, "").toLowerCase();
}

export function safeUrl(raw) {
  const u = String(raw || "").trim();
  if (!u || hasCtl(u)) return "";
  const compact = decodedCompact(u);
  if (
    compact.startsWith("javascript:") ||
    compact.startsWith("data:") ||
    compact.startsWith("vbscript:") ||
    compact.startsWith("javascript") && compact.includes(":")
  ) {
    return "";
  }
  if (compact.includes("javascript:") || compact.includes("data:") || compact.includes("vbscript:")) {
    return "";
  }
  if (u.startsWith("//")) return "";
  if (u.startsWith("/") && !u.startsWith("//")) return u;
  if (u.startsWith("#")) return u;
  if (compact.startsWith("https://") || compact.startsWith("http://") || compact.startsWith("mailto:")) {
    return u;
  }
  return "";
}

function parseAttrs(raw) {
  const out = [];
  const re = /([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/g;
  let m;
  while ((m = re.exec(raw))) {
    const name = m[1].toLowerCase();
    const val = m[2] ?? m[3] ?? m[4] ?? "";
    if (name.startsWith("on")) continue;
    if (!ALLOWED_ATTRS.has(name)) continue;
    if (name === "href") {
      const href = safeUrl(val);
      if (!href) continue;
      out.push(`href="${escapeText(href)}"`);
      continue;
    }
    out.push(`${name}="${escapeText(val)}"`);
  }
  return out;
}

/** Sanitize HTML fragment; unknown tags stripped, text kept. */
export function sanitizeHtml(html) {
  const src = String(html || "");
  if (!src) return "";
  return src.replace(/<\/?([a-zA-Z][a-zA-Z0-9]*)\b([^>]*)>|<!--[\s\S]*?-->/g, (full, tag, attrs) => {
    if (full.startsWith("<!--")) return "";
    const name = String(tag || "").toLowerCase();
    const closing = full.startsWith("</");
    const selfClosing = /\/\s*>$/.test(full) || name === "br" || name === "hr";
    if (!ALLOWED_TAGS.has(name)) return "";
    if (closing) return `</${name}>`;
    const attrStr = parseAttrs(attrs || "").join(" ");
    const open = attrStr ? `<${name} ${attrStr}` : `<${name}`;
    if (selfClosing && (name === "br" || name === "hr")) return `${open} />`;
    return `${open}>`;
  });
}

function sanitizeNode(node) {
  if (Array.isArray(node)) return node.map(sanitizeNode);
  if (!node || typeof node !== "object") return node;
  const next = Array.isArray(node) ? [] : { ...node };
  for (const [k, val] of Object.entries(node)) {
    if (k === "href" || k === "url") {
      next[k] = safeUrl(val) || "";
    } else if (k === "html" && (node.type === "richtext" || typeof val === "string")) {
      next[k] = node.type === "richtext" || k === "html" ? sanitizeHtml(String(val || "")) : val;
      if (node.type === "richtext") next[k] = sanitizeHtml(String(val || ""));
    } else {
      next[k] = sanitizeNode(val);
    }
  }
  if (next.type === "richtext") {
    next.html = sanitizeHtml(next.html || "");
  }
  return next;
}

export function sanitizeBlocks(body) {
  const b = body && typeof body === "object" ? body : { blocks: [] };
  const blocks = Array.isArray(b.blocks) ? b.blocks : [];
  return {
    ...b,
    blocks: blocks.map((block) => sanitizeNode(block)),
  };
}

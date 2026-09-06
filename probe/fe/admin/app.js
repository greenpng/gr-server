(function () {
  var BASE = (function () {
    var p = location.pathname.replace(/\/+$/, "");
    if (p.endsWith("/index.html")) p = p.slice(0, -11);
    return p || "";
  })();
  var TOKEN_KEY = "gr_admin_token";
  var state = { token: localStorage.getItem(TOKEN_KEY) || "", user: "", sites: [] };

  function api(method, path, body) {
    var opts = {
      method: method,
      headers: { "Content-Type": "application/json" },
    };
    if (state.token) {
      opts.headers["Authorization"] = "Bearer " + state.token;
      opts.headers["X-Session-Token"] = state.token;
    }
    if (body != null) opts.body = JSON.stringify(body);
    return fetch(BASE + "/api/" + path, opts).then(function (r) {
      return r.json().then(function (j) {
        if (r.status === 401) throw new Error("unauthorized");
        if (!r.ok && j && j.error) throw new Error(j.error);
        return j;
      });
    });
  }

  function esc(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function route() {
    var h = (location.hash || "#/dashboard").replace(/^#\/?/, "");
    var name = h.split("?")[0] || "dashboard";
    if (name === "sessions") name = "visits";
    return name;
  }

  function globalSiteId() {
    var el = document.getElementById("globalSite");
    return el && el.value ? el.value : "";
  }

  function fillSites(selId, includeAll) {
    return api("GET", "sites").then(function (j) {
      state.sites = j.sites || [];
      var sel = document.getElementById(selId || "globalSite");
      if (!sel) return state.sites;
      var cur = sel.value;
      sel.innerHTML = includeAll === false ? "" : '<option value="">全部站点</option>';
      state.sites.forEach(function (s) {
        var o = document.createElement("option");
        o.value = s.site_id;
        o.textContent = s.name + " (" + s.site_id + ")";
        sel.appendChild(o);
      });
      if (cur) sel.value = cur;
      return state.sites;
    });
  }

  function setNav(name) {
    document.querySelectorAll(".nav a").forEach(function (a) {
      a.classList.toggle("active", a.getAttribute("data-route") === name);
    });
    var titles = {
      dashboard: "业务看板 · 浏览器/Robots",
      sites: "站点",
      deploy: "部署中心 · nginx / CF / 注入 / 验证",
      domains: "域名绑定",
      ssl: "SSL 证书",
      runtime: "运行时（运维）",
      sdk: "SDK 嵌入与 Key",
      visits: "访客 / 访问",
      sessions: "访客 / 访问",
      system: "系统健康",
      config: "配置 · 全局默认",
      "config-site": "配置 · 站点覆盖",
      cluster: "集群节点",
      governance: "身份治理 · 合并/拆分运行时",
    };
    document.getElementById("pageTitle").textContent = titles[name] || name;
  }

  function showApp() {
    document.getElementById("loginGate").style.display = "none";
    document.getElementById("appShell").classList.add("ready");
    document.getElementById("userChip").textContent = state.user || "";
    fillSites("globalSite", true).then(render);
  }

  function render() {
    var name = route();
    setNav(name);
    var view = document.getElementById("view");
    view.innerHTML = '<div class="muted">加载中…</div>';
    var fn = pages[name] || pages.dashboard;
    Promise.resolve(fn(view)).catch(function (e) {
      view.innerHTML = '<div class="err">' + esc(e.message || e) + "</div>";
      if (String(e.message) === "unauthorized") {
        state.token = "";
        localStorage.removeItem(TOKEN_KEY);
        location.reload();
      }
    });
  }

  var pages = {};

  pages.dashboard = function (view) {
    var qs = "limit=800";
    var sid = globalSiteId();
    if (sid) qs += "&site_id=" + encodeURIComponent(sid);
    return api("GET", "stats/facets?" + qs).then(function (j) {
      var f = (j.facets || {});
      var leg = j.legacy_facets || {};
      var byName = j.robots_by_name || {};
      var nameRows = Object.keys(byName)
        .map(function (k) {
          return (
            "<tr><td>" +
            esc(k) +
            '</td><td class="num">' +
            esc(byName[k]) +
            "</td></tr>"
          );
        })
        .join("");
      var browserN = f.browser || 0;
      var robotsN = f.robots || 0;
      var binTotal = f.total != null ? f.total : browserN + robotsN;
      view.innerHTML =
        '<div class="note"><strong>业务看板 · 二分类</strong>（taxonomy: browser_robots_v2）。' +
        "新写入仅 <code>browser</code>（浏览器）/ <code>robots</code>。" +
        "历史 js/nojs <strong>不迁移</strong>，不计入下方二分类；可在「历史残留」查看。" +
        "数据来自 <code>gr_biz</code>，不进 commercial digest。</div>" +
        '<div class="grid">' +
        '<div class="stat browser"><div class="k">浏览器</div><div class="v">' +
        esc(browserN) +
        "</div><div class='muted'>UA 非爬虫名 · 含 FE / 网关 / 像素</div></div>" +
        '<div class="stat robots"><div class="k">Robots</div><div class="v">' +
        esc(robotsN) +
        "</div><div class='muted'>UA 含明确爬虫 / 工具名</div></div>" +
        '<div class="stat"><div class="k">二分类合计</div><div class="v">' +
        esc(binTotal) +
        "</div></div>" +
        '<div class="stat"><div class="k">扫描窗口</div><div class="v">' +
        esc(j.scanned || 0) +
        "</div><div class='muted'>含历史行</div></div>" +
        "</div>" +
        (nameRows
          ? '<div class="card"><h2>Robots 名称细分</h2><table class="tbl"><thead><tr><th>名称</th><th>次数</th></tr></thead><tbody>' +
            nameRows +
            "</tbody></table></div>"
          : "") +
        '<div class="card"><h2>历史残留（不迁移）</h2><div class="grid">' +
        '<div class="stat js"><div class="k">legacy js</div><div class="v">' +
        esc(leg.js || 0) +
        "</div></div>" +
        '<div class="stat nojs"><div class="k">legacy nojs</div><div class="v">' +
        esc(leg.nojs || 0) +
        "</div></div>" +
        '<div class="stat"><div class="k">unknown</div><div class="v">' +
        esc(leg.unknown || 0) +
        "</div></div></div>" +
        '<p class="muted">历史数据仅供对照；新流量只写 browser / robots。</p></div>' +
        '<div class="card"><h2>快捷入口</h2>' +
        '<div class="toolbar">' +
        '<button onclick="location.hash=\'#/visits?facet=browser\'">浏览器访问</button>' +
        '<button class="danger" onclick="location.hash=\'#/visits?facet=robots\'">Robots 访问</button>' +
        '<button class="secondary" onclick="location.hash=\'#/sites\'">管理站点</button>' +
        "</div></div>";
    });
  };

  pages.sites = function (view) {
    return api("GET", "sites").then(function (j) {
      var rows = j.sites || [];
      view.innerHTML =
        '<div class="note"><strong>部署模型（请按项选择）</strong><br/>' +
        "① <b>FE 加载</b>仅两种：一方 <code>/g5</code> 或独立 <code>pv</code> 域。<br/>" +
        "② <b>探测上传</b>三路：一方 <code>/g5</code> open/ingest · 可选 <code>pv</code> · <b>gv 永远必须</b>（Pingora TLS / B8，须 HTTPS）。" +
        " <code>/g5-gw</code> 只是 nginx 备份，<b>不能</b>替代独立 gv。<br/>" +
        "③ 浏览器直连上传，后端 SDK <b>不中转</b>探测包（只消费 result）。<br/>" +
        "④ 橙云 CF：同源请求须带 cookie（cf_clearance）；跨域 pv/gv 无法复用 www 的 clearance。</div>" +
        '<div class="card"><h2>新建 / 更新站点</h2>' +
        '<div class="row2"><div><label>名称</label><input id="siteName" /></div>' +
        '<div><label>备注</label><input id="siteNotes" /></div></div>' +
        '<div class="row2"><div><label>站点 ID（更新时填已有 id）</label><input id="siteId" placeholder="留空自动生成" /></div>' +
        '<div><label>轮询协议 poll_method</label><select id="sitePoll">' +
        '<option value="both">both · POST 优先再 GET（推荐）</option>' +
        '<option value="post">post · 仅 POST /analyses</option>' +
        '<option value="get">get · 仅 GET /analyses</option>' +
        "</select></div></div>" +
        '<div class="row2"><div><label>① FE 脚本加载 fe_load</label><select id="siteFeLoad">' +
        '<option value="first_party">一方 · /g5/dist（Under Attack 推荐）</option>' +
        '<option value="pv">pv · https://pv…/dist</option>' +
        '<option value="gv">gv · https://gv…/dist</option>' +
        "</select></div>" +
        '<div><label>② 上传路径 upload_ingest</label><select id="siteUpload">' +
        '<option value="gv">gv 统一 · https://gv…（推荐，上传+B8）</option>' +
        '<option value="first_party">一方 · /g5 → :28765</option>' +
        '<option value="pv">pv 域 · https://pv… → :28765</option>' +
        "</select></div></div>" +
        '<label>pv_base（fe_load 或 upload 选 pv 时必填 https://pv.example.com）</label>' +
        '<input id="sitePv" placeholder="https://pv.example.com（一方加载时可留空）" />' +
        '<label>gv_base（③ 必填 · Pingora HTTPS · 例 https://gv.example.com）</label>' +
        '<input id="siteGv" placeholder="https://gv.example.com" />' +
        '<div class="muted" style="margin:6px 0">gv 须单独 SSL（面板 SSL 页或外部证书）；建议 DNS-only / 不经 CF Under Attack。</div>' +
        '<div class="toolbar"><button id="btnSiteSave">保存站点</button>' +
        '<button id="btnSiteEdgeOnly" class="ok">保存拓扑</button>' +
        '<button id="btnSiteGuide" class="secondary">简要说明</button>' +
        '<button id="btnSiteDeploy" class="ok">打开部署中心</button></div>' +
        '<pre id="siteGuide" style="max-height:280px;overflow:auto;margin-top:10px"></pre></div>' +
        '<div class="card"><h2>站点列表</h2>' +
        '<div class="filters"><div><label>搜索</label><input id="siteQ" placeholder="名称 / id" /></div>' +
        '<div><button id="btnSiteFilter" class="secondary">筛选</button></div></div>' +
        '<table><thead><tr><th>名称</th><th>ID</th><th>加载/上传</th><th>pv / gv</th><th>采集</th><th>操作</th></tr></thead>' +
        '<tbody id="siteBody"></tbody></table></div>';
      function paint(list) {
        var tb = document.getElementById("siteBody");
        if (!list.length) {
          tb.innerHTML = '<tr><td colspan="6" class="empty">暂无站点</td></tr>';
          return;
        }
        tb.innerHTML = list
          .map(function (s) {
            var pv = s.pv_base || "";
            var gv = s.gv_base || "";
            var fe = s.fe_load || "first_party";
            var up = s.upload_ingest || "first_party";
            return (
              "<tr><td>" +
              esc(s.name) +
              "</td><td><code>" +
              esc(s.site_id) +
              "</code></td><td style=\"font-size:12px\"><code>load:" +
              esc(fe) +
              "</code><br/><code>up:" +
              esc(up) +
              "</code><br/><code>poll:" +
              esc(s.poll_method || "both") +
              "</code></td><td class=\"muted\" style=\"font-size:12px;max-width:220px;word-break:break-all\">" +
              "pv: " +
              esc(pv || "(一方 /g5)") +
              "<br/>gv: " +
              esc(gv || "⚠ 未配置") +
              "</td><td><span class=\"pill " +
              (s.collect_enabled ? "on" : "off") +
              '">' +
              (s.collect_enabled ? "开启" : "关闭") +
              "</span></td><td>" +
              '<button data-id="' +
              esc(s.site_id) +
              '" data-on="' +
              (s.collect_enabled ? "0" : "1") +
              '" class="secondary btnToggle" style="width:auto">切换</button> ' +
              '<button data-id="' +
              esc(s.site_id) +
              '" class="secondary btnEdit" style="width:auto">编辑拓扑</button> ' +
              '<button data-id="' +
              esc(s.site_id) +
              '" class="secondary btnGuide" style="width:auto">部署说明</button> ' +
              '<button data-id="' +
              esc(s.site_id) +
              '" class="secondary btnDom" style="width:auto">域名</button>' +
              "</td></tr>"
            );
          })
          .join("");
        tb.querySelectorAll(".btnToggle").forEach(function (b) {
          b.onclick = function () {
            api("POST", "sites/" + b.getAttribute("data-id") + "/toggle", {
              collect_enabled: b.getAttribute("data-on") === "1",
            }).then(function () {
              pages.sites(view);
              fillSites("globalSite", true);
            });
          };
        });
        tb.querySelectorAll(".btnDom").forEach(function (b) {
          b.onclick = function () {
            document.getElementById("globalSite").value = b.getAttribute("data-id");
            location.hash = "#/domains";
          };
        });
        tb.querySelectorAll(".btnEdit").forEach(function (b) {
          b.onclick = function () {
            var id = b.getAttribute("data-id");
            var s = rows.find(function (x) {
              return x.site_id === id;
            });
            if (!s) return;
            document.getElementById("siteId").value = s.site_id;
            document.getElementById("siteName").value = s.name || "";
            document.getElementById("siteNotes").value = s.notes || "";
            document.getElementById("siteFeLoad").value = s.fe_load || "first_party";
            document.getElementById("siteUpload").value = s.upload_ingest || "first_party";
            document.getElementById("sitePoll").value = s.poll_method || "both";
            document.getElementById("sitePv").value = s.pv_base || "";
            document.getElementById("siteGv").value = s.gv_base || "";
            window.scrollTo(0, 0);
          };
        });
        tb.querySelectorAll(".btnGuide").forEach(function (b) {
          b.onclick = function () {
            var id = b.getAttribute("data-id");
            api("GET", "sites/" + encodeURIComponent(id) + "/deploy-guide")
              .then(function (j) {
                document.getElementById("siteGuide").textContent = JSON.stringify(
                  j.deploy_guide || j,
                  null,
                  2
                );
                window.scrollTo(0, 0);
              })
              .catch(function (e) {
                document.getElementById("siteGuide").textContent = e.message || String(e);
              });
          };
        });
      }
      paint(rows);
      function showGuide(j) {
        var g = (j && j.deploy_guide) || j;
        document.getElementById("siteGuide").textContent = JSON.stringify(g, null, 2);
      }
      function edgePayload() {
        return {
          fe_load: document.getElementById("siteFeLoad").value,
          upload_ingest: document.getElementById("siteUpload").value,
          poll_method: document.getElementById("sitePoll").value,
          pv_base: document.getElementById("sitePv").value.trim(),
          gv_base: document.getElementById("siteGv").value.trim(),
        };
      }
      document.getElementById("btnSiteSave").onclick = function () {
        var payload = {
          name: document.getElementById("siteName").value,
          notes: document.getElementById("siteNotes").value,
        };
        var sid = document.getElementById("siteId").value.trim();
        if (sid) payload.site_id = sid;
        api("POST", "sites", payload)
          .then(function (r) {
            var id = (r.site && r.site.site_id) || sid;
            if (!id) {
              pages.sites(view);
              return;
            }
            document.getElementById("siteId").value = id;
            return api("POST", "sites/" + encodeURIComponent(id) + "/edge", edgePayload()).then(
              function (er) {
                showGuide(er);
                pages.sites(view);
                fillSites("globalSite", true);
              }
            );
          })
          .catch(function (e) {
            document.getElementById("siteGuide").textContent = e.message || String(e);
          });
      };
      document.getElementById("btnSiteEdgeOnly").onclick = function () {
        var sid = document.getElementById("siteId").value.trim();
        if (!sid) {
          alert("保存拓扑需要站点 ID（可从列表「编辑拓扑」填入，或先保存站点）");
          return;
        }
        api("POST", "sites/" + encodeURIComponent(sid) + "/edge", edgePayload())
          .then(function (er) {
            showGuide(er);
            pages.sites(view);
            fillSites("globalSite", true);
          })
          .catch(function (e) {
            document.getElementById("siteGuide").textContent = e.message || String(e);
          });
      };
      document.getElementById("btnSiteGuide").onclick = function () {
        var sid = document.getElementById("siteId").value.trim();
        if (!sid) {
          alert("请先选择站点 ID");
          return;
        }
        api("GET", "sites/" + encodeURIComponent(sid) + "/deploy-guide")
          .then(showGuide)
          .catch(function (e) {
            document.getElementById("siteGuide").textContent = e.message || String(e);
          });
      };
      document.getElementById("btnSiteDeploy").onclick = function () {
        var sid = document.getElementById("siteId").value.trim();
        if (sid) {
          try {
            document.getElementById("globalSite").value = sid;
          } catch (e0) {}
        }
        location.hash = "#/deploy";
      };
      document.getElementById("btnSiteFilter").onclick = function () {
        var q = document.getElementById("siteQ").value.trim().toLowerCase();
        paint(
          rows.filter(function (s) {
            return (
              !q ||
              (s.name + s.site_id + (s.edge_mode || "") + (s.pv_base || "") + (s.gv_base || ""))
                .toLowerCase()
                .indexOf(q) >= 0
            );
          })
        );
      };
    });
  };

  /**
   * Deploy Center — unified operator surface:
   * profile choice · nginx · Cloudflare · inject · site app · verify.
   */
  pages.deploy = function (view) {
    var sid = globalSiteId();
    return Promise.all([
      fillSites(null, true),
      api("GET", "deploy/profiles"),
      sid
        ? api("GET", "sites/" + encodeURIComponent(sid) + "/deploy").catch(function (e) {
            return { error: e.message || String(e) };
          })
        : Promise.resolve(null),
    ]).then(function (all) {
      var profiles = (all[1] && all[1].profiles) || [];
      var pack = all[2];
      var profileOpts = profiles
        .map(function (p) {
          return (
            '<option value="' +
            esc(p.id) +
            '">' +
            esc(p.name) +
            "</option>"
          );
        })
        .join("");

      view.innerHTML =
        '<div class="note"><strong>部署中心</strong>：按站点选择拓扑后，查看 <b>DNS/SSL · Nginx 脚本 · Cloudflare · 页面注入 · 站点程序 · 后端 SDK · 验证清单</b>。' +
        " 推荐：<code>fe_load=first_party</code>（nginx /g5 加载）+ <code>upload_ingest=gv</code>（open/ingest+B8 同走 https://gv，Pingora 直连，<b>禁止 nginx 反代 gv</b>）。</div>" +
        '<div class="card"><h2>1. 选择站点与部署配置文件</h2>' +
        '<div class="row2"><div><label>站点</label><select id="depSite"></select></div>' +
        '<div><label>配置模板 profile</label><select id="depProfile">' +
        profileOpts +
        "</select></div></div>" +
        '<label>gv_base（必填 HTTPS，Pingora / 统一上传）</label>' +
        '<input id="depGv" placeholder="https://gv.example.com" />' +
        '<label>pv_base（仅 fe_load/upload 用 pv 时）</label>' +
        '<input id="depPv" placeholder="https://pv.example.com（可选）" />' +
        '<div class="toolbar">' +
        '<button id="btnDepLoad" class="secondary">加载部署包</button>' +
        '<button id="btnDepApply" class="ok">应用模板到站点</button>' +
        '<button id="btnDepVerify" class="ok">运行服务端验证</button>' +
        '<button id="btnDepCopy" class="secondary">复制当前 JSON</button></div>' +
        '<div class="muted" id="depHint" style="margin-top:8px"></div></div>' +
        '<div class="card"><h2>2. 拓扑一览</h2>' +
        '<pre id="depTopo" style="white-space:pre-wrap"></pre></div>' +
        '<div class="card"><h2>3. 分节说明</h2>' +
        '<div class="filters" style="flex-wrap:wrap;gap:6px" id="depTabs"></div>' +
        '<div id="depSection"></div></div>' +
        '<div class="card"><h2>4. 验证结果</h2><pre id="depVerify" style="max-height:240px;overflow:auto"></pre></div>' +
        '<div class="card"><h2>5. 完整部署包 JSON</h2>' +
        '<pre id="depJson" style="max-height:360px;overflow:auto"></pre></div>';

      fillSites("depSite", false);
      var depSel = document.getElementById("depSite");
      if (sid && depSel) depSel.value = sid;

      function paintPack(p) {
        if (!p || p.error) {
          document.getElementById("depTopo").textContent = (p && p.error) || "请选择站点并加载";
          document.getElementById("depJson").textContent = "";
          document.getElementById("depSection").innerHTML = "";
          document.getElementById("depTabs").innerHTML = "";
          return;
        }
        var topo = p.topology || {};
        document.getElementById("depTopo").textContent =
          (topo.ascii || "") +
          "\nprofile: " +
          (p.profile_id || "") +
          " — " +
          (p.profile_label || "") +
          "\napiBase=" +
          (topo.api_base || "") +
          "  gwBase=" +
          (topo.gw_base || "") +
          "  script=" +
          (topo.script_base || "") +
          "\nunified_gv=" +
          !!topo.unified_gv_upload;
        if (p.resolved) {
          if (p.resolved.gv_base) document.getElementById("depGv").value = p.resolved.gv_base;
          if (p.resolved.pv_base) document.getElementById("depPv").value = p.resolved.pv_base || "";
        }
        if (p.profile_id) {
          try {
            document.getElementById("depProfile").value = p.profile_id;
          } catch (e1) {}
        }
        document.getElementById("depJson").textContent = JSON.stringify(p, null, 2);

        var sections = p.sections || {};
        var keys = Object.keys(sections);
        var tabs = document.getElementById("depTabs");
        var order = [
          "overview",
          "dns_ssl",
          "nginx",
          "cloudflare",
          "inject",
          "site_app",
          "backend_sdk",
          "verify",
          "checklist",
        ];
        keys.sort(function (a, b) {
          var ia = order.indexOf(a);
          var ib = order.indexOf(b);
          if (ia < 0) ia = 99;
          if (ib < 0) ib = 99;
          return ia - ib;
        });
        var labels = {
          overview: "总览",
          dns_ssl: "DNS/SSL",
          nginx: "Nginx/脚本",
          cloudflare: "Cloudflare",
          inject: "页面注入",
          site_app: "站点程序",
          backend_sdk: "后端 SDK",
          verify: "验证清单",
          checklist: "上线勾选",
        };
        tabs.innerHTML = keys
          .map(function (k, i) {
            return (
              '<button type="button" class="secondary depTab' +
              (i === 0 ? " ok" : "") +
              '" data-k="' +
              esc(k) +
              '" style="width:auto">' +
              esc(labels[k] || k) +
              "</button>"
            );
          })
          .join("");

        function showSec(k) {
          tabs.querySelectorAll(".depTab").forEach(function (b) {
            b.classList.toggle("ok", b.getAttribute("data-k") === k);
          });
          var sec = sections[k] || {};
          var html = "<h3>" + esc(sec.title || k) + "</h3>";
          if (k === "checklist" && sec.items) {
            html +=
              "<ul style=\"line-height:1.7\">" +
              sec.items
                .map(function (it) {
                  var mark =
                    it.ok === true ? "✅" : it.ok === false ? "❌" : it.manual ? "☐" : "·";
                  return "<li>" + mark + " " + esc(it.label || it.id) + "</li>";
                })
                .join("") +
              "</ul>";
          } else if (k === "nginx") {
            html += "<p class=\"muted\">执行顺序（生产机 root）</p><ol>";
            (sec.steps || []).forEach(function (s) {
              html += "<li style=\"margin-bottom:8px\"><code style=\"white-space:pre-wrap\">" + esc(s) + "</code></li>";
            });
            html += "</ol>";
            if (sec.snippets) {
              html += "<p><b>Nginx 片段</b></p>";
              Object.keys(sec.snippets).forEach(function (sk) {
                html +=
                  "<div class=\"muted\">" +
                  esc(sk) +
                  "</div><pre style=\"max-height:200px;overflow:auto\">" +
                  esc(sec.snippets[sk]) +
                  "</pre>";
              });
            }
            if (sec.script_commands) {
              html +=
                "<p><b>脚本命令</b></p><pre>" +
                esc(sec.script_commands.join("\n")) +
                "</pre>";
            }
          } else if (k === "inject") {
            (sec.methods || []).forEach(function (m) {
              html +=
                "<div style=\"margin:10px 0;padding:8px;border:1px solid #333;border-radius:6px\">" +
                "<b>" +
                esc(m.name || m.id) +
                "</b>" +
                (m.command ? "<pre>" + esc(m.command) + "</pre>" : "") +
                (m.snippet
                  ? "<pre style=\"max-height:220px;overflow:auto\">" + esc(m.snippet) + "</pre>"
                  : "") +
                (m.note ? "<div class=\"muted\">" + esc(m.note) + "</div>" : "") +
                (m.doc ? "<div class=\"muted\">doc: " + esc(m.doc) + "</div>" : "") +
                "</div>";
            });
            if (sec.boot_fields) {
              html +=
                "<p><b>Boot 字段</b></p><pre>" +
                esc(JSON.stringify(sec.boot_fields, null, 2)) +
                "</pre>";
            }
          } else if (k === "cloudflare") {
            html += "<p><b>DNS 代理</b></p><table><thead><tr><th>host</th><th>proxy</th><th>Under Attack</th><th>说明</th></tr></thead><tbody>";
            (sec.dns_proxy || []).forEach(function (r) {
              html +=
                "<tr><td><code>" +
                esc(r.host) +
                "</code></td><td>" +
                esc(r.proxy) +
                "</td><td>" +
                esc(r.under_attack) +
                "</td><td class=\"muted\">" +
                esc(r.note) +
                "</td></tr>";
            });
            html += "</tbody></table>";
            if (sec.under_attack) {
              html +=
                "<p><b>I'm Under Attack</b></p><pre>" +
                esc(JSON.stringify(sec.under_attack, null, 2)) +
                "</pre>";
            }
            if (sec.waf_optional_skip) {
              html +=
                "<p><b>可选 WAF Skip</b></p><pre>" +
                esc(JSON.stringify(sec.waf_optional_skip, null, 2)) +
                "</pre>";
            }
            if (sec.headers_and_ssl) {
              html +=
                "<ul>" +
                sec.headers_and_ssl
                  .map(function (x) {
                    return "<li>" + esc(x) + "</li>";
                  })
                  .join("") +
                "</ul>";
            }
            if (sec.cf_worker) {
              html +=
                "<p><b>CF Worker</b></p><pre>" +
                esc(JSON.stringify(sec.cf_worker, null, 2)) +
                "</pre>";
            }
          } else if (k === "verify") {
            html += "<p><b>服务端命令</b></p><pre>" + esc((sec.server_commands || []).join("\n")) + "</pre>";
            html +=
              "<p><b>浏览器清单</b></p><ol>" +
              (sec.browser_checklist || [])
                .map(function (x) {
                  return "<li>" + esc(x) + "</li>";
                })
                .join("") +
              "</ol>";
            if (sec.fail_matrix) {
              html +=
                "<p><b>故障对照</b></p><table><thead><tr><th>现象</th><th>处理</th></tr></thead><tbody>" +
                sec.fail_matrix
                  .map(function (f) {
                    return (
                      "<tr><td>" +
                      esc(f.symptom) +
                      "</td><td>" +
                      esc(f.fix) +
                      "</td></tr>"
                    );
                  })
                  .join("") +
                "</tbody></table>";
            }
            html +=
              "<p class=\"muted\">" +
              esc(sec.cf_cookie_check || "") +
              " · " +
              esc(sec.panel_verify_api || "") +
              "</p>";
          } else {
            html +=
              "<pre style=\"max-height:420px;overflow:auto\">" +
              esc(JSON.stringify(sec, null, 2)) +
              "</pre>";
          }
          document.getElementById("depSection").innerHTML = html;
        }

        tabs.querySelectorAll(".depTab").forEach(function (b) {
          b.onclick = function () {
            showSec(b.getAttribute("data-k"));
          };
        });
        if (keys.length) showSec(keys[0]);
        state._deployPack = p;
      }

      function currentSid() {
        return (document.getElementById("depSite") && document.getElementById("depSite").value) || "";
      }

      document.getElementById("btnDepLoad").onclick = function () {
        var id = currentSid();
        if (!id) {
          document.getElementById("depHint").textContent = "请选择站点";
          return;
        }
        try {
          document.getElementById("globalSite").value = id;
        } catch (e2) {}
        api("GET", "sites/" + encodeURIComponent(id) + "/deploy")
          .then(function (p) {
            document.getElementById("depHint").textContent = "已加载 " + id;
            paintPack(p);
          })
          .catch(function (e) {
            document.getElementById("depHint").textContent = e.message || String(e);
          });
      };

      document.getElementById("btnDepApply").onclick = function () {
        var id = currentSid();
        if (!id) {
          alert("请选择站点");
          return;
        }
        var body = {
          profile_id: document.getElementById("depProfile").value,
          gv_base: document.getElementById("depGv").value.trim(),
          pv_base: document.getElementById("depPv").value.trim(),
        };
        api("POST", "sites/" + encodeURIComponent(id) + "/deploy-profile", body)
          .then(function (r) {
            document.getElementById("depHint").textContent =
              "已应用模板 " + (r.profile_id || "") + " → 站点 " + id;
            if (r.deploy) paintPack(r.deploy);
            else return api("GET", "sites/" + encodeURIComponent(id) + "/deploy").then(paintPack);
          })
          .catch(function (e) {
            document.getElementById("depHint").textContent = e.message || String(e);
          });
      };

      document.getElementById("btnDepVerify").onclick = function () {
        var id = currentSid();
        if (!id) {
          alert("请选择站点");
          return;
        }
        document.getElementById("depVerify").textContent = "验证中…";
        api("POST", "sites/" + encodeURIComponent(id) + "/deploy-verify", {})
          .then(function (r) {
            document.getElementById("depVerify").textContent = JSON.stringify(r, null, 2);
            var s = r.summary || {};
            document.getElementById("depHint").textContent =
              "验证完成 pass=" + (s.pass || 0) + "/" + (s.total || 0);
          })
          .catch(function (e) {
            document.getElementById("depVerify").textContent = e.message || String(e);
          });
      };

      document.getElementById("btnDepCopy").onclick = function () {
        var t = document.getElementById("depJson").textContent || "";
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(t).then(
            function () {
              document.getElementById("depHint").textContent = "已复制部署包 JSON";
            },
            function () {
              document.getElementById("depHint").textContent = "复制失败，请手动选择";
            }
          );
        }
      };

      // profile description on change
      document.getElementById("depProfile").onchange = function () {
        var id = document.getElementById("depProfile").value;
        var p = profiles.find(function (x) {
          return x.id === id;
        });
        document.getElementById("depHint").textContent = p
          ? p.summary || p.name
          : "";
      };
      if (profiles[0]) {
        document.getElementById("depHint").textContent = profiles[0].summary || "";
      }

      if (pack && !pack.error) paintPack(pack);
      else if (sid) document.getElementById("btnDepLoad").click();
    });
  };

  pages.domains = function (view) {
    var sid = globalSiteId();
    var path = "domains" + (sid ? "?site_id=" + encodeURIComponent(sid) : "");
    return Promise.all([fillSites(null, true), api("GET", path)]).then(function (all) {
      var domains = all[1].domains || [];
      view.innerHTML =
        '<div class="card"><h2>绑定域名</h2>' +
        '<div class="row2"><div><label>归属站点</label><select id="domSite"></select></div>' +
        '<div><label>hostname</label><input id="domHost" placeholder="probe.example.com" /></div></div>' +
        '<div><label>显示名</label><input id="domName" /></div>' +
        '<button id="btnDomSave">添加 / 更新</button></div>' +
        '<div class="card"><h2>域名列表</h2>' +
        '<div class="filters"><div><label>搜索</label><input id="domQ" /></div>' +
        '<div><button id="btnDomFilter" class="secondary">筛选</button></div></div>' +
        '<table><thead><tr><th>hostname</th><th>站点</th><th>SSL</th><th>采集</th><th>操作</th></tr></thead>' +
        '<tbody id="domBody"></tbody></table></div>';
      var sel = document.getElementById("domSite");
      sel.innerHTML = "";
      state.sites.forEach(function (s) {
        var o = document.createElement("option");
        o.value = s.site_id;
        o.textContent = s.name;
        sel.appendChild(o);
      });
      if (sid) sel.value = sid;
      function paint(list) {
        var tb = document.getElementById("domBody");
        if (!list.length) {
          tb.innerHTML = '<tr><td colspan="5" class="empty">暂无域名</td></tr>';
          return;
        }
        tb.innerHTML = list
          .map(function (d) {
            return (
              "<tr><td>" +
              esc(d.hostname) +
              "</td><td><code>" +
              esc(d.site_id) +
              "</code></td><td>" +
              esc(d.ssl_status) +
              '</td><td><span class="pill ' +
              (d.collect_enabled ? "on" : "off") +
              '">' +
              (d.collect_enabled ? "开" : "关") +
              "</span></td><td>" +
              '<button class="secondary btnT" data-id="' +
              esc(d.domain_id) +
              '" data-on="' +
              (d.collect_enabled ? "0" : "1") +
              '" style="width:auto">切换</button> ' +
              '<button class="secondary btnSsl" data-id="' +
              esc(d.domain_id) +
              '" style="width:auto">SSL</button>' +
              "</td></tr>"
            );
          })
          .join("");
        tb.querySelectorAll(".btnT").forEach(function (b) {
          b.onclick = function () {
            api("POST", "domains/" + b.getAttribute("data-id") + "/toggle", {
              collect_enabled: b.getAttribute("data-on") === "1",
            }).then(function () {
              pages.domains(view);
            });
          };
        });
        tb.querySelectorAll(".btnSsl").forEach(function (b) {
          b.onclick = function () {
            location.hash = "#/ssl?domain_id=" + encodeURIComponent(b.getAttribute("data-id"));
          };
        });
      }
      paint(domains);
      document.getElementById("btnDomSave").onclick = function () {
        api("POST", "domains", {
          site_id: document.getElementById("domSite").value,
          hostname: document.getElementById("domHost").value,
          display_name: document.getElementById("domName").value,
        }).then(function () {
          pages.domains(view);
        });
      };
      document.getElementById("btnDomFilter").onclick = function () {
        var q = document.getElementById("domQ").value.trim().toLowerCase();
        paint(
          domains.filter(function (d) {
            return !q || (d.hostname + d.display_name + d.domain_id).toLowerCase().indexOf(q) >= 0;
          })
        );
      };
    });
  };

  pages.ssl = function (view) {
    var params = new URLSearchParams((location.hash.split("?")[1] || ""));
    var pref = params.get("domain_id") || "";
    return api("GET", "domains").then(function (j) {
      var domains = j.domains || [];
      view.innerHTML =
        '<div class="note">支持自签、粘贴 PEM、Let\'s Encrypt HTTP-01（默认 staging）。证书写入管理库并热载入 SNI。</div>' +
        '<div class="card"><h2>选择域名</h2><select id="sslDom"></select>' +
        '<div class="toolbar" style="margin-top:10px">' +
        '<button id="btnSelf" class="ok">签发自签</button>' +
        '<button id="btnPrep" class="secondary">准备 ACME challenge</button>' +
        '<button id="btnLe">申请 Let\'s Encrypt</button>' +
        "</div>" +
        '<div class="row2"><div><label>邮箱 (LE)</label><input id="leEmail" placeholder="admin@example.com" /></div>' +
        '<div><label>生产目录</label><select id="leProd"><option value="0">Staging</option><option value="1">Production</option></select></div></div>' +
        '<label>粘贴 fullchain PEM</label><textarea id="pemCert" rows="5"></textarea>' +
        '<label>粘贴 privkey PEM</label><textarea id="pemKey" rows="4"></textarea>' +
        '<button id="btnPem" class="secondary">保存 PEM 并应用 SNI</button>' +
        '<pre id="sslOut"></pre></div>';
      var sel = document.getElementById("sslDom");
      domains.forEach(function (d) {
        var o = document.createElement("option");
        o.value = d.domain_id;
        o.textContent = d.hostname + " · " + d.ssl_status;
        sel.appendChild(o);
      });
      if (pref) sel.value = pref;
      function out(x) {
        document.getElementById("sslOut").textContent =
          typeof x === "string" ? x : JSON.stringify(x, null, 2);
      }
      function id() {
        return document.getElementById("sslDom").value;
      }
      document.getElementById("btnSelf").onclick = function () {
        api("POST", "domains/" + id() + "/ssl/self-signed", {}).then(out).catch(function (e) {
          out(e.message);
        });
      };
      document.getElementById("btnPrep").onclick = function () {
        api("POST", "domains/" + id() + "/ssl/prepare-challenge", {}).then(out).catch(function (e) {
          out(e.message);
        });
      };
      document.getElementById("btnLe").onclick = function () {
        api("POST", "domains/" + id() + "/ssl/issue-le", {
          email: document.getElementById("leEmail").value,
          production: document.getElementById("leProd").value === "1",
        })
          .then(out)
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnPem").onclick = function () {
        api("POST", "domains/" + id() + "/ssl/save", {
          cert_pem: document.getElementById("pemCert").value,
          key_pem: document.getElementById("pemKey").value,
          set_active_edge: true,
        })
          .then(out)
          .catch(function (e) {
            out(e.message);
          });
      };
    });
  };

  function numInput(id, label, val, hint) {
    return (
      "<div><label>" +
      esc(label) +
      (hint ? ' <span class="muted">' + esc(hint) + "</span>" : "") +
      '</label><input type="number" id="' +
      id +
      '" value="' +
      esc(val == null ? "" : val) +
      '" /></div>'
    );
  }

  function readNum(id) {
    var el = document.getElementById(id);
    if (!el || el.value === "") return null;
    var n = Number(el.value);
    return isFinite(n) ? n : null;
  }

  pages.config = function (view) {
    return api("GET", "config").then(function (j) {
      var g = j.global || {};
      var live = j.live || {};
      var def = j.defaults || {};
      var help = g.field_help || def.field_help || {};
      function h(key) {
        return help[key] ? help[key] : "默认 " + (def[key] != null ? def[key] : "");
      }
      view.innerHTML =
        '<div class="note"><strong>业务参数热配</strong>（非算法）：超时 / 冷热 / 重试 / 冷却。' +
        "保存草稿 → <strong>发布</strong> 升 <code>config_version</code>；节点 ≤30s reload。" +
        "密钥/DSN/算法红线 <strong>不上屏</strong>。return idle ≥10s。</div>" +
        '<div class="grid">' +
        '<div class="stat"><div class="k">草稿 version</div><div class="v">' +
        esc(g.version || 0) +
        "</div></div>" +
        '<div class="stat"><div class="k">已生效 live</div><div class="v">' +
        esc(live.version || 0) +
        '</div><div class="muted">actor ' +
        esc(live.actor || "") +
        "</div></div>" +
        '<div class="stat"><div class="k">更新 ms</div><div class="v" style="font-size:14px">' +
        esc(g.updated_ms || live.updated_ms || "—") +
        "</div></div></div>" +
        '<div class="card"><h2>周期 / 会话</h2><div class="row2">' +
        numInput("cfgCool", "cycle_cool_ms", g.cycle_cool_ms, h("cycle_cool_ms")) +
        numInput("cfgInc", "cycle_incomplete_ms", g.cycle_incomplete_ms, h("cycle_incomplete_ms")) +
        numInput("cfgSessIn", "session_inactivity_ms", g.session_inactivity_ms, h("session_inactivity_ms")) +
        numInput("cfgSessHard", "session_hard_max_ms", g.session_hard_max_ms, h("session_hard_max_ms")) +
        "</div></div>" +
        '<div class="card"><h2>热 / 冷存储</h2><div class="row2">' +
        numInput("cfgHot", "hot_idle_ms", g.hot_idle_ms, h("hot_idle_ms")) +
        numInput("cfgColdTtl", "cold_ttl_ms", g.cold_ttl_ms, h("cold_ttl_ms")) +
        numInput("cfgColdProm", "cold_promote_window_ms", g.cold_promote_window_ms, h("cold_promote_window_ms")) +
        numInput("cfgPurge", "cold_purge_interval_ms", g.cold_purge_interval_ms, h("cold_purge_interval_ms")) +
        "</div></div>" +
        '<div class="card"><h2>分析 / 返回 / RPA</h2><div class="row2">' +
        numInput("cfgAnIdle", "analyze_idle_upload_ms", g.analyze_idle_upload_ms, h("analyze_idle_upload_ms")) +
        numInput("cfgAnDeb", "analyze_debounce_ms", g.analyze_debounce_ms, h("analyze_debounce_ms")) +
        numInput("cfgRetIdle", "return_identity_idle_ms", g.return_identity_idle_ms, h("return_identity_idle_ms")) +
        numInput("cfgRpa", "rpa_idle_analyze_ms", g.rpa_idle_analyze_ms, h("rpa_idle_analyze_ms")) +
        "</div></div>" +
        '<div class="card"><h2>FE 重试 / 预算 / SLA</h2><p class="muted">经 open.policy.fe_retry 下发到 probe_lifecycle（发布后新会话生效）。</p><div class="row2">' +
        numInput("cfgHardAtt", "hard_max_attempts", g.hard_max_attempts, h("hard_max_attempts")) +
        numInput("cfgSoftAtt", "soft_max_attempts", g.soft_max_attempts, h("soft_max_attempts")) +
        numInput("cfgDeepAtt", "deepen_max_attempts", g.deepen_max_attempts, h("deepen_max_attempts")) +
        numInput("cfgRpaAtt", "rpa_max_attempts", g.rpa_max_attempts, h("rpa_max_attempts")) +
        numInput("cfgFailN", "fail_budget_n", g.fail_budget_n, h("fail_budget_n")) +
        numInput("cfgFailW", "fail_budget_window_ms", g.fail_budget_window_ms, h("fail_budget_window_ms")) +
        numInput("cfgRpaQ", "rpa_quiet_ms", g.rpa_quiet_ms, h("rpa_quiet_ms")) +
        numInput("cfgSlaN", "hard_sla_retries", g.hard_sla_retries, h("hard_sla_retries")) +
        numInput("cfgSlaD", "hard_sla_base_delay_ms", g.hard_sla_base_delay_ms, h("hard_sla_base_delay_ms")) +
        numInput("cfgTick", "multi_tick_max", g.multi_tick_max, h("multi_tick_max")) +
        numInput("cfgEmpty", "empty_kick_patience", g.empty_kick_patience, h("empty_kick_patience")) +
        numInput("cfgUpMax", "upload_max_retries", g.upload_max_retries, h("upload_max_retries")) +
        numInput("cfgAlive", "client_alive_retry_ms", g.client_alive_retry_ms, h("client_alive_retry_ms")) +
        numInput("cfgUpConc", "upload_concurrency", g.upload_concurrency, h("upload_concurrency")) +
        numInput("cfgUpMid", "upload_mid_ramp", g.upload_mid_ramp, h("upload_mid_ramp")) +
        numInput("cfgUpRamp", "upload_ramp_after", g.upload_ramp_after, h("upload_ramp_after")) +
        "</div></div>" +
        '<div class="card"><h2>关周期策略</h2><div class="row2">' +
        '<div><label>complete_on_commercial_silicon <span class="muted">' +
        esc(h("complete_on_commercial_silicon")) +
        '</span></label><select id="cfgCompleteSi"><option value="true">true（默认，推荐）</option><option value="false">false</option></select></div>' +
        "</div></div>" +
        '<div class="toolbar">' +
        '<button id="btnCfgSave">保存草稿</button>' +
        '<button id="btnCfgPub" class="ok">发布并应用</button>' +
        '<button id="btnCfgReload" class="secondary">重新加载</button></div>' +
        '<pre id="cfgOut"></pre>';
      document.getElementById("cfgCompleteSi").value =
        g.complete_on_commercial_silicon === false ? "false" : "true";
      function body() {
        var o = {};
        var map = {
          cycle_cool_ms: "cfgCool",
          cycle_incomplete_ms: "cfgInc",
          session_inactivity_ms: "cfgSessIn",
          session_hard_max_ms: "cfgSessHard",
          hot_idle_ms: "cfgHot",
          cold_ttl_ms: "cfgColdTtl",
          cold_promote_window_ms: "cfgColdProm",
          cold_purge_interval_ms: "cfgPurge",
          analyze_idle_upload_ms: "cfgAnIdle",
          analyze_debounce_ms: "cfgAnDeb",
          return_identity_idle_ms: "cfgRetIdle",
          rpa_idle_analyze_ms: "cfgRpa",
          hard_max_attempts: "cfgHardAtt",
          soft_max_attempts: "cfgSoftAtt",
          deepen_max_attempts: "cfgDeepAtt",
          rpa_max_attempts: "cfgRpaAtt",
          fail_budget_n: "cfgFailN",
          fail_budget_window_ms: "cfgFailW",
          rpa_quiet_ms: "cfgRpaQ",
          hard_sla_retries: "cfgSlaN",
          hard_sla_base_delay_ms: "cfgSlaD",
          multi_tick_max: "cfgTick",
          empty_kick_patience: "cfgEmpty",
          upload_max_retries: "cfgUpMax",
          client_alive_retry_ms: "cfgAlive",
          upload_concurrency: "cfgUpConc",
          upload_mid_ramp: "cfgUpMid",
          upload_ramp_after: "cfgUpRamp",
        };
        Object.keys(map).forEach(function (k) {
          var v = readNum(map[k]);
          if (v != null) o[k] = v;
        });
        o.complete_on_commercial_silicon =
          document.getElementById("cfgCompleteSi").value === "true";
        return o;
      }
      function out(x) {
        document.getElementById("cfgOut").textContent =
          typeof x === "string" ? x : JSON.stringify(x, null, 2);
      }
      document.getElementById("btnCfgSave").onclick = function () {
        api("PUT", "config/global", body())
          .then(out)
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnCfgPub").onclick = function () {
        api("PUT", "config/global", body())
          .then(function () {
            return api("POST", "config/publish", {});
          })
          .then(function (r) {
            out(r);
            pages.config(view);
          })
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnCfgReload").onclick = function () {
        pages.config(view);
      };
    });
  };

  pages["config-site"] = function (view) {
    return Promise.all([api("GET", "config"), fillSites(null, true)]).then(function (all) {
      var sitesCfg = (all[0] && all[0].sites) || {};
      var sid0 = globalSiteId() || (state.sites[0] && state.sites[0].site_id) || "";
      view.innerHTML =
        '<div class="note">站点覆盖：空字段 = 继承全局。清空并保存可删除覆盖。发布后全局 version 提升，节点 reload 后 effective 合并。</div>' +
        '<div class="card"><h2>选择站点</h2>' +
        '<div class="row2"><div><label>site_id</label><select id="cfgSiteSel"></select></div>' +
        '<div><label>effective 预览</label><button id="btnEff" class="secondary" style="margin-top:22px">加载 effective</button></div></div>' +
        '<pre id="effOut" class="muted">选站点后查看合并结果</pre></div>' +
        '<div class="card"><h2>覆盖字段（留空=继承）</h2><div class="row2">' +
        numInput("sCool", "cycle_cool_ms", "", "") +
        numInput("sCold", "cold_ttl_ms", "", "") +
        numInput("sAn", "analyze_idle_upload_ms", "", "") +
        numInput("sRet", "return_identity_idle_ms", "", "min 10000") +
        numInput("sRpa", "rpa_idle_analyze_ms", "", "") +
        '<div><label>collect_enabled</label><select id="sCol"><option value="">继承</option><option value="true">true</option><option value="false">false</option></select></div>' +
        "</div>" +
        '<div class="toolbar"><button id="btnSiteCfgSave">保存站点覆盖</button>' +
        '<button id="btnSiteCfgClear" class="danger">清除覆盖</button>' +
        '<button id="btnSiteCfgPub" class="ok">发布全局</button></div>' +
        '<pre id="siteCfgOut"></pre></div>' +
        '<div class="card"><h2>已有覆盖</h2><pre id="sitesList"></pre></div>';
      var sel = document.getElementById("cfgSiteSel");
      state.sites.forEach(function (s) {
        var o = document.createElement("option");
        o.value = s.site_id;
        o.textContent = s.name + " (" + s.site_id + ")";
        sel.appendChild(o);
      });
      if (sid0) sel.value = sid0;
      document.getElementById("sitesList").textContent = JSON.stringify(sitesCfg, null, 2);
      function loadOv() {
        var sid = sel.value;
        var ov = sitesCfg[sid] || {};
        document.getElementById("sCool").value = ov.cycle_cool_ms != null ? ov.cycle_cool_ms : "";
        document.getElementById("sCold").value = ov.cold_ttl_ms != null ? ov.cold_ttl_ms : "";
        document.getElementById("sAn").value =
          ov.analyze_idle_upload_ms != null ? ov.analyze_idle_upload_ms : "";
        document.getElementById("sRet").value =
          ov.return_identity_idle_ms != null ? ov.return_identity_idle_ms : "";
        document.getElementById("sRpa").value =
          ov.rpa_idle_analyze_ms != null ? ov.rpa_idle_analyze_ms : "";
        document.getElementById("sCol").value =
          ov.collect_enabled === true ? "true" : ov.collect_enabled === false ? "false" : "";
      }
      loadOv();
      sel.onchange = loadOv;
      function out(x) {
        document.getElementById("siteCfgOut").textContent =
          typeof x === "string" ? x : JSON.stringify(x, null, 2);
      }
      function patchBody(clear) {
        if (clear) return { site_id: sel.value, override: {} };
        var ov = {};
        var c = readNum("sCool");
        if (c != null) ov.cycle_cool_ms = c;
        var d = readNum("sCold");
        if (d != null) ov.cold_ttl_ms = d;
        var a = readNum("sAn");
        if (a != null) ov.analyze_idle_upload_ms = a;
        var r = readNum("sRet");
        if (r != null) ov.return_identity_idle_ms = r;
        var p = readNum("sRpa");
        if (p != null) ov.rpa_idle_analyze_ms = p;
        var col = document.getElementById("sCol").value;
        if (col === "true") ov.collect_enabled = true;
        if (col === "false") ov.collect_enabled = false;
        return { site_id: sel.value, override: ov };
      }
      document.getElementById("btnSiteCfgSave").onclick = function () {
        api("PUT", "config/site", patchBody(false))
          .then(function (r) {
            out(r);
            return api("GET", "config");
          })
          .then(function (j) {
            sitesCfg = j.sites || {};
            document.getElementById("sitesList").textContent = JSON.stringify(sitesCfg, null, 2);
          })
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnSiteCfgClear").onclick = function () {
        api("PUT", "config/site", patchBody(true))
          .then(function (r) {
            out(r);
            pages["config-site"](view);
          })
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnSiteCfgPub").onclick = function () {
        api("POST", "config/publish", {})
          .then(out)
          .catch(function (e) {
            out(e.message);
          });
      };
      document.getElementById("btnEff").onclick = function () {
        api("GET", "config/effective?site_id=" + encodeURIComponent(sel.value))
          .then(function (j) {
            document.getElementById("effOut").textContent = JSON.stringify(j, null, 2);
          })
          .catch(function (e) {
            document.getElementById("effOut").textContent = e.message;
          });
      };
    });
  };

  pages.cluster = function (view) {
    return api("GET", "cluster/nodes").then(function (j) {
      var nodes = j.nodes || [];
      view.innerHTML =
        '<div class="note">节点心跳：挂载同一 admin 库的进程每 ~30s 上报。超过 15 分钟无心跳会从列表剔除。只读。</div>' +
        '<div class="card"><div class="toolbar"><button id="btnClRefresh" class="secondary">刷新</button>' +
        '<span class="muted">n=' +
        esc(j.n || nodes.length) +
        "</span></div>" +
        '<table><thead><tr><th>worker</th><th>role</th><th>cfg_ver</th><th>product</th><th>backend</th><th>soft</th><th>hot_vts</th><th>analyze_runs</th><th>last_beat</th></tr></thead><tbody>' +
        (nodes.length
          ? nodes
              .map(function (n) {
                return (
                  "<tr><td><code>" +
                  esc(n.worker_id) +
                  "</code></td><td>" +
                  esc(n.role) +
                  "</td><td>" +
                  esc(n.config_version) +
                  "</td><td>" +
                  esc(n.product_version) +
                  "</td><td>" +
                  esc(n.backend) +
                  "</td><td>" +
                  esc(n.soft_backend) +
                  "</td><td class='num'>" +
                  esc(n.hot_vts) +
                  "</td><td class='num'>" +
                  esc(n.analyze_runs) +
                  "</td><td>" +
                  esc(n.last_beat_ms) +
                  "</td></tr>"
                );
              })
              .join("")
          : '<tr><td colspan="9" class="empty">暂无心跳（确认 admin 已启动且进程共享 admin-db）</td></tr>') +
        "</tbody></table></div>";
      document.getElementById("btnClRefresh").onclick = function () {
        pages.cluster(view);
      };
    });
  };

  // iss/61 G5: 身份治理看板（iss/58-61 治理栈只读视图：共享锁/L2/m/u 普查/HNSW/对比编码/FS 阈值/conf 采用）。
  pages.governance = function (view) {
    return api("GET", "ops/identity_governance").then(function (j) {
      var sm = j.shared_metrics || {};
      var l2 = j.l2 || {};
      var census = (j.mu_census && j.mu_census.slots) || {};
      var hnsw = j.hnsw || {};
      var contra = j.contrastive || {};
      var taus = j.fs_taus_adopted || null;
      var conf = j.confidence || {};
      var adopt = conf.adopt || {};
      function ratio(a, b) {
        a = Number(a) || 0;
        b = Number(b) || 0;
        return b > 0 ? ((a / b) * 100).toFixed(1) + "%" : "—";
      }
      function stat(cls, k, v, sub) {
        return (
          '<div class="stat ' + cls + '"><div class="k">' + esc(k) + '</div><div class="v">' +
          esc(v) + "</div>" + (sub ? "<div class='muted'>" + esc(sub) + "</div>" : "") + "</div>"
        );
      }
      var slotRows = Object.keys(census)
        .sort()
        .map(function (name) {
          var s = census[name] || {};
          var top = (s.top || [])
            .map(function (t) {
              return "<code>" + esc(String(t.digest || "").slice(0, 12)) + "</code>×" + esc(t.n);
            })
            .join(" ");
          return (
            "<tr><td><code>" + esc(name) + "</code></td><td class='num'>" + esc(s.n || 0) +
            "</td><td class='num'>" + esc(ratio(s.same_agree, s.same_total)) +
            " <span class='muted'>(" + esc(s.same_agree || 0) + "/" + esc(s.same_total || 0) + ")</span></td>" +
            "<td class='num'>" + esc(s.unique_digests || 0) + "</td>" +
            "<td class='num'>" + esc(s.m_hat != null ? s.m_hat : "—") + "</td>" +
            "<td class='num'>" + esc(s.u_hat != null ? s.u_hat : "—") + "</td>" +
            "<td>" + (top || '<span class="muted">—</span>') + "</td></tr>"
          );
        })
        .join("");
      var hist = (hnsw.layer_hist || [])
        .map(function (n, lv) {
          return "L" + lv + ":" + n;
        })
        .join(" ");
      view.innerHTML =
        '<div class="note"><strong>身份治理</strong>（<code>' + esc(j.algo || "") + "</code> · v" +
        esc(j.product_version || "") + "）。合并/拆分治理运行时状态：共享治理锁与 Redis L2、Fellegi–Sunter m/u 普查、HNSW 近邻图、有监督对比编码器、FS 阈值与 conf 校准采用。只读；断言永不硬封（仅降权/加深/临时）。</div>" +
        '<div class="toolbar"><button id="btnGovRefresh" class="secondary">刷新</button></div>' +
        '<div class="grid">' +
        stat("", "共享治理锁", "ok " + (sm.lock_ok || 0) + " / fail " + (sm.lock_fail || 0), "save_fail " + (sm.save_fail || 0)) +
        stat(l2.enabled && l2.url_configured ? "browser" : "", "Redis L2", l2.enabled ? (l2.url_configured ? "on" : "on·未配 url") : "off", "ok " + (l2.ok || 0) + " / fail " + (l2.fail || 0)) +
        stat("", "FS 阈值(采用)", taus ? "m=" + taus.tau_merge + " s=" + taus.tau_split : "默认", taus ? "conf_cal 已采用" : "未采用·用内置默认") +
        stat("", "conf 运行版本", conf.runtime_version || "—", adopt.decision || adopt.status || "") +
        "</div>" +
        '<div class="card"><h2>m/u 普查（Fellegi–Sunter 槽位）</h2>' +
        '<table><thead><tr><th>slot</th><th>n</th><th>same 一致率</th><th>unique digests</th><th>m̂</th><th>û</th><th>top digests</th></tr></thead><tbody>' +
        (slotRows || '<tr><td colspan="7" class="empty">暂无普查数据（有分析流量后生成）</td></tr>') +
        "</tbody></table></div>" +
        '<div class="grid">' +
        '<div class="card"><h2>HNSW 近邻图</h2><table><tbody>' +
        "<tr><td>algo</td><td><code>" + esc(hnsw.algo || "") + "</code></td></tr>" +
        "<tr><td>nodes</td><td class='num'>" + esc(hnsw.nodes != null ? hnsw.nodes : "—") + "</td></tr>" +
        "<tr><td>max_level</td><td class='num'>" + esc(hnsw.max_level != null ? hnsw.max_level : "—") + "</td></tr>" +
        "<tr><td>emb_dim / M / Mmax0 / efC</td><td class='num'>" + esc(hnsw.emb_dim) + " / " + esc(hnsw.m) + " / " + esc(hnsw.m_max0) + " / " + esc(hnsw.ef_construction) + "</td></tr>" +
        "<tr><td>layer_hist</td><td><code>" + esc(hist || "—") + "</code></td></tr>" +
        "</tbody></table></div>" +
        '<div class="card"><h2>对比编码器（有监督）</h2><table><tbody>' +
        "<tr><td>algo</td><td><code>" + esc(contra.algo || "") + "</code></td></tr>" +
        "<tr><td>trained_steps</td><td class='num'>" + esc(contra.trained_steps || 0) + (contra.is_identity ? " <span class='muted'>(identity·未训练)</span>" : "") + "</td></tr>" +
        "<tr><td>pos / neg 对</td><td class='num'>" + esc(contra.n_pos || 0) + " / " + esc(contra.n_neg || 0) + "</td></tr>" +
        "<tr><td>last_loss</td><td class='num'>" + esc(contra.last_loss != null ? contra.last_loss : "—") + "</td></tr>" +
        "<tr><td>emb_dim / hidden / deep</td><td class='num'>" + esc(contra.emb_dim) + " / " + esc(contra.hidden) + " / " + esc(contra.deep) + "</td></tr>" +
        "<tr><td>shared / l2</td><td>" + esc(!!contra.shared) + " / " + esc(!!contra.l2) + "</td></tr>" +
        "</tbody></table></div>" +
        "</div>" +
        ((j.notes || []).length
          ? '<div class="card"><h2>说明</h2><ul>' +
            j.notes
              .map(function (n) {
                return "<li>" + esc(n) + "</li>";
              })
              .join("") +
            "</ul></div>"
          : "");
      document.getElementById("btnGovRefresh").onclick = function () {
        pages.governance(view);
      };
    });
  };

  pages.runtime = function (view) {
    return api("GET", "runtime").then(function (j) {
      var d = j.desired || {};
      view.innerHTML =
        '<div class="note">端口 / role 变更写入期望配置；应用时热载 SNI 与 analyze_workers。监听端口变更需重启（可配 GR_ADMIN_RESTART_CMD）。</div>' +
        '<div class="card"><h2>期望配置</h2>' +
        '<div class="row2"><div><label>bind</label><input id="rtBind" value="' +
        esc(d.bind || "") +
        '" /></div><div><label>bind_tls</label><input id="rtTls" value="' +
        esc(d.bind_tls || "") +
        '" /></div></div>' +
        '<div class="row2"><div><label>admin_bind</label><input id="rtAdmin" value="' +
        esc(d.admin_bind || "") +
        '" /></div><div><label>role</label><input id="rtRole" value="' +
        esc(d.role || "all") +
        '" /></div></div>' +
        '<div class="row2"><div><label>analyze_workers</label><input id="rtWorkers" type="number" value="' +
        esc(d.analyze_workers || 2) +
        '" /></div><div><label>soft_v2_ready</label><select id="rtSoft"><option value="false">false</option><option value="true">true</option></select></div></div>' +
        '<div class="toolbar"><button id="btnRtSave">保存</button><button id="btnRtApply" class="ok">应用</button></div>' +
        '<h2>当前实况</h2><pre id="rtLive"></pre></div>';
      document.getElementById("rtSoft").value = String(!!d.soft_v2_ready);
      document.getElementById("rtLive").textContent = JSON.stringify(j.live || {}, null, 2);
      document.getElementById("btnRtSave").onclick = function () {
        api("PUT", "runtime", {
          desired: {
            bind: document.getElementById("rtBind").value,
            bind_tls: document.getElementById("rtTls").value,
            admin_bind: document.getElementById("rtAdmin").value,
            role: document.getElementById("rtRole").value,
            analyze_workers: Number(document.getElementById("rtWorkers").value || 2),
            soft_v2_ready: document.getElementById("rtSoft").value === "true",
          },
        }).then(function () {
          pages.runtime(view);
        });
      };
      document.getElementById("btnRtApply").onclick = function () {
        api("POST", "runtime/apply", {}).then(function (r) {
          document.getElementById("rtLive").textContent = JSON.stringify(r, null, 2);
        });
      };
    });
  };

  pages.sdk = function (view) {
    var sid = globalSiteId();
    // Sensible defaults for local formal / production operators
    var hostNoPort = location.hostname || "127.0.0.1";
    var defaultApi = "http://" + hostNoPort + ":28765";
    var defaultGw = "http://" + hostNoPort + ":28766";
    return Promise.all([
      fillSites(null, true),
      api("GET", "sdk/keys" + (sid ? "?site_id=" + encodeURIComponent(sid) : "")),
    ]).then(function (all) {
      var keys = all[1].keys || [];
      view.innerHTML =
        '<div class="note"><strong>部署与嵌入</strong>：三种采集拓扑——' +
        '<b>first_party</b>（一方 <code>/g5</code>）、' +
        '<b>hybrid</b>（一方采数 + 独立 <code>gv</code>）、' +
        '<b>dual_domain</b>（独立 <code>pv</code>+<code>gv</code>，抗 CF Under Attack）。' +
        " 站点页可保存默认 pv/gv；此处生成片段时优先用站点配置。" +
        " 探测数据由浏览器直连 pv/gv，<b>无需后端 SDK 中转上传</b>；backend Key 只用于 result 消费。" +
        ' 文档：<code>v5-docs2/07-deploy/09-fe-load-first-party-and-cdn.md</code></div>' +
        '<div class="card"><h2>签发 Key</h2>' +
        '<div class="row2"><div><label>站点</label><select id="sdkSite"></select></div>' +
        '<div><label>类型</label><select id="sdkKind"><option value="backend">backend（业务后端）</option><option value="fe_embed">fe_embed（仅前缀展示）</option></select></div></div>' +
        '<label>allowed_origins（逗号分隔，backend 可选约束）</label><input id="sdkOrigins" placeholder="https://www.example.com,http://127.0.0.1:18090" />' +
        '<div class="toolbar"><button id="btnSdkCreate">签发</button>' +
        '<button id="btnEnforceOn" class="ok">开启 enforce</button>' +
        '<button id="btnEnforceOff" class="secondary">关闭 enforce</button></div>' +
        '<pre id="sdkOut"></pre></div>' +
        '<div class="card"><h2>生成前端嵌入片段</h2>' +
        '<div class="row2"><div><label>部署模式</label><select id="embedMode">' +
        '<option value="first_party">first_party · 一方 /g5</option>' +
        '<option value="hybrid">hybrid · 一方采数 + 独立 gv</option>' +
        '<option value="dual_domain">dual_domain · 独立 pv + gv</option>' +
        "</select></div>" +
        '<div><label>站点</label><select id="embedSite"></select></div></div>' +
        '<label>pv / API public_base（一方 /g5；dual 填 https://pv.example.com；可留空=用站点配置）</label>' +
        '<input id="embedApi" placeholder="/g5 或 https://pv.example.com" value="" />' +
        '<label>gv / Gateway gw_base（一方 /g5-gw；独立 https://gv.example.com）</label>' +
        '<input id="embedGw" placeholder="/g5-gw 或 https://gv.example.com" value="" />' +
        '<div class="toolbar">' +
        '<button id="btnEmbedFp" class="ok">生成 · first_party</button>' +
        '<button id="btnEmbedHy" class="secondary">生成 · hybrid</button>' +
        '<button id="btnEmbedX" class="secondary">生成 · dual_domain</button>' +
        '<button id="btnLoadSiteEdge" class="secondary">载入站点拓扑</button>' +
        '<button id="btnCopySnippet" class="secondary">复制 snippet</button></div>' +
        '<div class="muted" id="embedHint" style="margin:8px 0"></div>' +
        '<pre id="embedOut" style="max-height:360px;overflow:auto"></pre></div>' +
        '<div class="card"><h2>使用后怎么看数据</h2>' +
        "<ol style=\"margin:0 0 0 1.2em;line-height:1.6\">" +
        "<li><b>访客/访问</b>：本控制台左侧「访客 / 访问」— vtid、session、sdk_synced</li>" +
        "<li><b>业务看板</b>：browser / robots 分面统计</li>" +
        "<li><b>探测深挖</b>：左侧「探测会话 ops」或 <code>/ops.html#&lt;session_id&gt;</code></li>" +
        "<li><b>业务后端</b>：SDK <code>session_consumption</code> / <code>GET /v1/session/:id/result</code></li>" +
        "</ol></div>" +
        '<div class="card"><h2>Key 列表</h2><table><thead><tr><th>prefix</th><th>站点</th><th>类型</th><th>状态</th><th>操作</th></tr></thead>' +
        '<tbody id="sdkBody"></tbody></table></div>';
      function fillSiteSelect(elId) {
        var sel = document.getElementById(elId);
        if (!sel) return;
        sel.innerHTML = "";
        state.sites.forEach(function (s) {
          var o = document.createElement("option");
          o.value = s.site_id;
          o.textContent = s.name + " (" + s.site_id + ")";
          sel.appendChild(o);
        });
        if (sid) sel.value = sid;
      }
      fillSiteSelect("sdkSite");
      fillSiteSelect("embedSite");
      // defaults by mode
      function applyModeDefaults() {
        var mode = document.getElementById("embedMode").value;
        var apiEl = document.getElementById("embedApi");
        var gwEl = document.getElementById("embedGw");
        var hint = document.getElementById("embedHint");
        if (mode === "first_party") {
          if (!apiEl.value || apiEl.value.indexOf("http") === 0) apiEl.value = "/g5";
          if (!gwEl.value || gwEl.value.indexOf("http") === 0) gwEl.value = "/g5-gw";
          hint.innerHTML =
            "一方：客户 www 装 <code>install_nginx_g5_first_party.sh</code> 反代 /g5，再注入本片段。" +
            " 主路径同源；若主站开 CF Under Attack，API 会一并 challenge — 攻击期改 dual_domain。";
        } else if (mode === "hybrid") {
          if (!apiEl.value || apiEl.value.indexOf("http") === 0) apiEl.value = "/g5";
          if (!gwEl.value || gwEl.value === "/g5-gw") gwEl.value = defaultGw.replace(":28765", ":28766") || defaultGw;
          hint.innerHTML =
            "hybrid：采数仍 /g5；gw 填独立 <code>https://gv.example.com</code>（建议不经 CF）。" +
            " 可点「载入站点拓扑」使用站点页已保存的 gv。";
        } else {
          if (!apiEl.value || apiEl.value === "/g5") apiEl.value = defaultApi;
          if (!gwEl.value || gwEl.value === "/g5-gw") gwEl.value = defaultGw;
          hint.innerHTML =
            "dual_domain：pv=静态+ingest，gv=gateway；业务站可严格 CF，采集与主站解耦。" +
            " <b>必须</b>登记业务 hostname；pv/gv 保存站点拓扑时会自动登记探测主机。" +
            " 生产 <code>GR_CORS_ORIGINS=admin</code>。";
        }
      }
      document.getElementById("embedMode").onchange = applyModeDefaults;
      applyModeDefaults();
      document.getElementById("btnLoadSiteEdge").onclick = function () {
        var site = document.getElementById("embedSite").value;
        if (!site) return;
        api("GET", "sites/" + encodeURIComponent(site) + "/edge")
          .then(function (j) {
            var r = (j && j.resolved) || {};
            var s = (j && j.site) || {};
            var mode = r.edge_mode || s.edge_mode || "first_party";
            document.getElementById("embedMode").value = mode;
            document.getElementById("embedApi").value = r.api_base || s.pv_base || "";
            document.getElementById("embedGw").value = r.gw_base || s.gv_base || "";
            applyModeDefaults();
            document.getElementById("embedHint").textContent =
              "已载入站点拓扑 edge_mode=" + mode + " api=" + (r.api_base || "") + " gw=" + (r.gw_base || "");
          })
          .catch(function (e) {
            document.getElementById("embedHint").textContent = e.message || String(e);
          });
      };

      var tb = document.getElementById("sdkBody");
      tb.innerHTML = keys.length
        ? keys
            .map(function (k) {
              return (
                "<tr><td>" +
                esc(k.secret_prefix) +
                "</td><td>" +
                esc(k.site_id) +
                "</td><td>" +
                esc(k.kind) +
                "</td><td>" +
                esc(k.status) +
                '</td><td><button class="danger btnRev" data-id="' +
                esc(k.key_id) +
                '" style="width:auto">吊销</button></td></tr>'
              );
            })
            .join("")
        : '<tr><td colspan="5" class="empty">暂无 key</td></tr>';
      tb.querySelectorAll(".btnRev").forEach(function (b) {
        b.onclick = function () {
          api("POST", "sdk/keys/" + b.getAttribute("data-id") + "/revoke", {}).then(function () {
            pages.sdk(view);
          });
        };
      });
      function out(x) {
        document.getElementById("sdkOut").textContent =
          typeof x === "string" ? x : JSON.stringify(x, null, 2);
      }
      function embedOut(x) {
        var el = document.getElementById("embedOut");
        if (typeof x === "string") {
          el.textContent = x;
          return;
        }
        // Prefer showing snippet first for operators
        var pretty = {
          mode: x.mode,
          api_base: x.api_base,
          gw_base: x.gw_base,
          boot_url: x.boot_url,
          inject_path: x.inject_path,
          snippet: x.snippet,
          deploy_notes: x.deploy_notes,
          how_to_view_data: x.how_to_view_data,
        };
        el.textContent = JSON.stringify(pretty, null, 2);
        el.dataset.snippet = x.snippet || "";
      }
      document.getElementById("btnSdkCreate").onclick = function () {
        var origins = document
          .getElementById("sdkOrigins")
          .value.split(",")
          .map(function (s) {
            return s.trim();
          })
          .filter(Boolean);
        api("POST", "sdk/keys", {
          site_id: document.getElementById("sdkSite").value,
          kind: document.getElementById("sdkKind").value,
          allowed_origins: origins,
        })
          .then(function (r) {
            out(r);
            pages.sdk(view);
          })
          .catch(function (e) {
            out(e.message);
          });
      };
      function doEmbed(forceMode) {
        var site = document.getElementById("embedSite").value;
        var mode = forceMode || document.getElementById("embedMode").value;
        // Map legacy label
        if (mode === "cross_origin" || mode === "cdn") mode = "dual_domain";
        document.getElementById("embedMode").value = mode;
        var apiBase = document.getElementById("embedApi").value.trim();
        var gwBase = document.getElementById("embedGw").value.trim();
        if (mode === "first_party" && !apiBase) apiBase = "/g5";
        if (mode === "hybrid" && !apiBase) apiBase = "/g5";
        if (mode === "dual_domain" && apiBase && apiBase.indexOf("http") !== 0) {
          embedOut("dual_domain 需要绝对 pv/public_base（https://pv.example.com）；可留空用站点配置");
          // still allow empty → server uses site config
        }
        var q =
          "sdk/embed?site_id=" +
          encodeURIComponent(site) +
          "&mode=" +
          encodeURIComponent(mode);
        if (apiBase) q += "&public_base=" + encodeURIComponent(apiBase);
        if (gwBase) q += "&gw_base=" + encodeURIComponent(gwBase);
        api("GET", q)
          .then(embedOut)
          .catch(function (e) {
            embedOut(e.message);
          });
      }
      document.getElementById("btnEmbedFp").onclick = function () {
        doEmbed("first_party");
      };
      document.getElementById("btnEmbedHy").onclick = function () {
        doEmbed("hybrid");
      };
      document.getElementById("btnEmbedX").onclick = function () {
        doEmbed("dual_domain");
      };
      document.getElementById("btnCopySnippet").onclick = function () {
        var sn =
          document.getElementById("embedOut").dataset.snippet ||
          document.getElementById("embedOut").textContent;
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(sn).then(
            function () {
              document.getElementById("embedHint").textContent = "已复制到剪贴板";
            },
            function () {
              document.getElementById("embedHint").textContent = "复制失败，请手动选中 pre";
            }
          );
        } else {
          document.getElementById("embedHint").textContent = "请手动选中下方 snippet 复制";
        }
      };
      document.getElementById("btnEnforceOn").onclick = function () {
        api("POST", "sdk/enforce", { enabled: true }).then(out);
      };
      document.getElementById("btnEnforceOff").onclick = function () {
        api("POST", "sdk/enforce", { enabled: false }).then(out);
      };
    });
  };

  pages.visits = function (view) {
    var params = new URLSearchParams((location.hash.split("?")[1] || ""));
    var facet = params.get("facet") || "";
    var sid = globalSiteId();
    view.innerHTML =
      '<div class="note"><strong>轻量访问列表</strong>（配置与运维视角，非算法仓库）。' +
      "二分类 browser/robots；<code>sdk_synced</code> 表示后端 SDK 是否把 product 摘要同步到 gr_biz。" +
      "多会话关联/策略数据请用 <strong>SDK 源码</strong>；算法深挖见 ops。</div>" +
      '<div class="card"><div class="filters">' +
      '<div><label>facet</label><select id="sessFacet"><option value="">全部</option>' +
      '<option value="browser">浏览器 browser</option>' +
      '<option value="robots">robots</option>' +
      '<option value="js">历史 js</option><option value="nojs">历史 nojs</option></select></div>' +
      '<div><label>搜索 vtid / session</label><input id="sessQ" /></div>' +
      '<div><button id="btnSess">查询</button></div></div>' +
      '<table><thead><tr><th>visit</th><th>site</th><th>vtid</th><th>facet</th>' +
      "<th>消费</th><th>device_id</th><th>更新</th><th></th></tr></thead>" +
      '<tbody id="sessBody"><tr><td colspan="8" class="muted">加载中…</td></tr></tbody></table></div>';
    document.getElementById("sessFacet").value = facet;
    function load() {
      var f = document.getElementById("sessFacet").value;
      var q = document.getElementById("sessQ").value.trim();
      var path =
        "visits?limit=50" +
        (f ? "&facet=" + encodeURIComponent(f) : "") +
        (sid ? "&site_id=" + encodeURIComponent(sid) : "") +
        (q ? "&q=" + encodeURIComponent(q) : "");
      return api("GET", path).then(function (j) {
        var rows = j.visits || j.sessions || [];
        var tb = document.getElementById("sessBody");
        if (!rows.length) {
          tb.innerHTML = '<tr><td colspan="8" class="empty">无匹配访问</td></tr>';
          return;
        }
        tb.innerHTML = rows
          .map(function (s) {
            var opsLink = s.session_id
              ? '<a target="_blank" href="/ops.html#' + esc(s.session_id) + '">ops</a>'
              : "";
            var lite = s.product_lite || {};
            var did = lite.device_id || (s.summary && s.summary.device_id) || "";
            var cons = s.consumption_state || "none";
            var syncPill = s.sdk_synced
              ? '<span class="pill on">sdk_synced</span>'
              : '<span class="pill off">' + esc(cons) + "</span>";
            return (
              "<tr><td><code>" +
              esc(s.visit_id || "") +
              "</code></td><td>" +
              esc(s.site_id || "") +
              "</td><td><code>" +
              esc(s.visitor_terminal_id || "") +
              '</code></td><td><span class="pill ' +
              esc(s.visitor_facet) +
              '">' +
              esc(s.visitor_facet) +
              "</span></td><td>" +
              syncPill +
              "</td><td><code>" +
              esc(String(did).slice(0, 28)) +
              (String(did).length > 28 ? "…" : "") +
              "</code></td><td>" +
              esc(s.updated_ms) +
              "</td><td>" +
              opsLink +
              "</td></tr>"
            );
          })
          .join("");
      });
    }
    document.getElementById("btnSess").onclick = load;
    return load();
  };

  pages.sessions = pages.visits;

  pages.system = function (view) {
    return Promise.all([
      api("GET", "system"),
      api("GET", "audit?limit=30"),
      api("GET", "ops/client-events-policy").catch(function () {
        return { ops_client_events_enabled: true, ok: false };
      }),
      api("GET", "cluster/nodes").catch(function () {
        return { nodes: [] };
      }),
    ]).then(function (all) {
      var sys = all[0] || {};
      var pol = all[2] || {};
      var en = pol.ops_client_events_enabled !== false;
      var storage = sys.storage || {};
      var purge = sys.last_cold_purge || {};
      var live = sys.config_live || {};
      var nodes = (all[3] && all[3].nodes) || [];
      view.innerHTML =
        '<div class="note">运维视图：配置库 / 看板库路径与健康。探测深挖见 ops。' +
        "客户端错误日志上传默认开启，可在下方关闭（不写库）。DSN/密钥仅脱敏展示。</div>" +
        '<div class="grid">' +
        '<div class="stat"><div class="k">config_version</div><div class="v">' +
        esc(sys.config_version != null ? sys.config_version : live.version || 0) +
        "</div></div>" +
        '<div class="stat"><div class="k">集群节点</div><div class="v">' +
        esc(nodes.length) +
        '</div><div class="muted"><a href="#/cluster">详情</a></div></div>' +
        '<div class="stat"><div class="k">soft backend</div><div class="v" style="font-size:16px">' +
        esc(storage.soft_store_backend || "—") +
        "</div></div>" +
        '<div class="stat"><div class="k">上次 cold purge</div><div class="v" style="font-size:14px">' +
        esc(purge.deleted != null ? purge.deleted + " rows" : "—") +
        '</div><div class="muted">at ' +
        esc(purge.at_ms || "—") +
        "</div></div></div>" +
        '<div class="card"><h2>存储健康（脱敏）</h2><table><tbody>' +
        "<tr><td>greenv5</td><td>" +
        (storage.greenv5_configured ? '<span class="pill on">on</span> ' : '<span class="pill off">off</span> ') +
        "<code>" +
        esc(storage.greenv5_dsn) +
        "</code></td></tr>" +
        "<tr><td>biz</td><td>" +
        (storage.biz_configured ? '<span class="pill on">on</span> ' : '<span class="pill off">off</span> ') +
        "<code>" +
        esc(storage.biz_dsn) +
        "</code></td></tr>" +
        "<tr><td>association</td><td>" +
        (storage.association_configured ? '<span class="pill on">on</span> ' : '<span class="pill off">off</span> ') +
        "<code>" +
        esc(storage.association_dsn) +
        "</code></td></tr>" +
        "<tr><td>redis</td><td>" +
        (storage.redis_configured ? '<span class="pill on">on</span> ' : '<span class="pill off">off</span> ') +
        "<code>" +
        esc(storage.redis_dsn || "—") +
        "</code></td></tr>" +
        "<tr><td>secrets</td><td>challenge=" +
        (storage.challenge_secret_configured ? "✓" : "—") +
        " seal=" +
        (storage.seal_secret_configured ? "✓" : "—") +
        " result_token=" +
        (storage.result_token_configured ? "✓" : "—") +
        "（永不显示明文）</td></tr>" +
        "</tbody></table></div>" +
        '<div class="card"><h2>客户端错误日志上传</h2>' +
        '<p class="muted">FE <code>ops_report.js</code> → <code>POST /v1/ops/client_event</code>。完整时间线见 <a href="/ops.html" target="_blank">/ops.html</a>。</p>' +
        '<div class="toolbar"><button id="btnOpsOn" class="ok">开启上传</button>' +
        '<button id="btnOpsOff" class="danger">关闭上传</button>' +
        '<span class="muted" id="opsPolLabel">当前: ' +
        (en ? "开启" : "关闭") +
        "</span></div></div>" +
        '<div class="card"><h2>日志中心（轻量）</h2>' +
        '<p class="muted">鉴权后读 probe 库 ops 事件（SQLite/PG 均可）。source=client|server。</p>' +
        '<div class="toolbar">' +
        '<select id="opsLogSrc"><option value="server">server</option><option value="client">client</option></select>' +
        '<button id="btnOpsLogLoad">加载</button>' +
        '<button id="btnOpsLogExport">导出 JSON</button></div>' +
        '<div id="opsLogBox" class="muted" style="margin-top:8px;max-height:280px;overflow:auto"></div></div>' +
        '<div class="card"><h2>系统 JSON</h2><pre>' +
        esc(JSON.stringify(sys, null, 2)) +
        '</pre></div><div class="card"><h2>审计日志</h2><table><thead><tr><th>时间</th><th>用户</th><th>动作</th><th>目标</th></tr></thead><tbody>' +
        (all[1].entries || [])
          .map(function (e) {
            return (
              "<tr><td>" +
              esc(e.ms) +
              "</td><td>" +
              esc(e.actor) +
              "</td><td>" +
              esc(e.action) +
              "</td><td>" +
              esc(e.target) +
              "</td></tr>"
            );
          })
          .join("") +
        "</tbody></table></div>";
      function setPol(on) {
        api("POST", "ops/client-events-policy", { enabled: on }).then(function (j) {
          document.getElementById("opsPolLabel").textContent =
            "当前: " + (j.ops_client_events_enabled ? "开启" : "关闭");
        });
      }
      document.getElementById("btnOpsOn").onclick = function () {
        setPol(true);
      };
      document.getElementById("btnOpsOff").onclick = function () {
        setPol(false);
      };
      function renderOpsLog(j) {
        var box = document.getElementById("opsLogBox");
        var evs = (j && j.events) || (j && j.client && j.client.events) || [];
        if (j && j.server && j.client) {
          box.innerHTML =
            "<p>client=" +
            ((j.client.events || []).length) +
            " · server=" +
            ((j.server.events || []).length) +
            " · exported_at_ms=" +
            esc(j.exported_at_ms) +
            "</p><pre style='font-size:11px;white-space:pre-wrap'>" +
            esc(
              JSON.stringify(
                {
                  client: (j.client.events || []).slice(0, 20),
                  server: (j.server.events || []).slice(0, 20),
                },
                null,
                2
              )
            ) +
            "</pre>";
          return;
        }
        if (!evs.length && j && j.events) evs = j.events;
        box.innerHTML =
          "<table><thead><tr><th>ts</th><th>code</th><th>sev</th><th>session</th><th>ver</th></tr></thead><tbody>" +
          (evs || [])
            .slice(0, 80)
            .map(function (e) {
              return (
                "<tr><td>" +
                esc(e.ts_ms || e.server_recv_ms) +
                "</td><td>" +
                esc(e.code) +
                "</td><td>" +
                esc(e.severity) +
                "</td><td>" +
                esc((e.session_id || "").slice(0, 16)) +
                "</td><td>" +
                esc((e.product_version || "").slice(0, 20)) +
                "</td></tr>"
              );
            })
            .join("") +
          "</tbody></table>";
      }
      document.getElementById("btnOpsLogLoad").onclick = function () {
        var src = document.getElementById("opsLogSrc").value || "server";
        api("GET", "ops/events?source=" + encodeURIComponent(src) + "&limit=100").then(function (j) {
          renderOpsLog(j);
        });
      };
      document.getElementById("btnOpsLogExport").onclick = function () {
        api("GET", "ops/events/export?limit=500").then(function (j) {
          renderOpsLog(j);
          var blob = new Blob([JSON.stringify(j, null, 2)], { type: "application/json" });
          var a = document.createElement("a");
          a.href = URL.createObjectURL(blob);
          a.download = "gr-ops-events-" + Date.now() + ".json";
          a.click();
          setTimeout(function () {
            URL.revokeObjectURL(a.href);
          }, 2000);
        });
      };
    });
  };

  document.getElementById("btnLogin").onclick = function () {
    var u = document.getElementById("loginUser").value;
    var p = document.getElementById("loginPass").value;
    document.getElementById("loginErr").textContent = "";
    api("POST", "auth/login", { username: u, password: p })
      .then(function (j) {
        state.token = j.token;
        state.user = j.username;
        localStorage.setItem(TOKEN_KEY, j.token);
        showApp();
      })
      .catch(function (e) {
        document.getElementById("loginErr").textContent = e.message || "login failed";
      });
  };
  document.getElementById("btnLogout").onclick = function () {
    api("POST", "auth/logout", {}).finally(function () {
      state.token = "";
      localStorage.removeItem(TOKEN_KEY);
      location.reload();
    });
  };
  document.getElementById("globalSite").onchange = render;
  window.addEventListener("hashchange", render);

  if (state.token) {
    api("GET", "auth/me")
      .then(function (j) {
        state.user = j.username;
        showApp();
      })
      .catch(function () {
        state.token = "";
        localStorage.removeItem(TOKEN_KEY);
      });
  }
})();

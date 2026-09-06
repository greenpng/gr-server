/**
 * Multi-sandbox tree (ported design from v57 sandbox_tree — in-tree, no path dep).
 * kinds: iframe | sandbox_iframe | worker  (blob_iframe via nest_frame)
 *
 * RPA: each surface binds its OWN continuous monitors (nest_frame / worker code).
 * Parent only enqueues B11 from real child flushes — never re-tags main events.
 */
(function (global) {
  "use strict";

  function enqueue(Q, row) {
    if (Q && Q.enqueue) Q.enqueue(row);
  }

  function pageUrlOf(ctx) {
    try {
      return (ctx && ctx.page_url) || String(location.href || "");
    } catch (e) {
      return "";
    }
  }

  function pageIdOf(ctx) {
    return (ctx && (ctx.page_id || (ctx.fields && ctx.fields.page_id))) || null;
  }

  /** Keep nest iframes alive so continuous RPA can flush (do not remove on first result). */
  var liveNests = [];

  /** Accumulate nest_* compare fields for B7 summary (never pollute main residual). */
  function noteNestCompare(fields) {
    if (!fields || typeof fields !== "object") return;
    var acc = global.__GR_NEST_COMPARE__ || {};
    if (fields.nest_residual_mean != null) acc.nest_residual_mean = fields.nest_residual_mean;
    if (fields.nest_engine_family) acc.nest_engine_family = fields.nest_engine_family;
    else if (fields.engine_family) acc.nest_engine_family = fields.engine_family;
    else if (fields.sandbox_kind) acc.nest_engine_family = String(fields.sandbox_kind);
    if (fields.user_agent) acc.nest_user_agent = String(fields.user_agent).slice(0, 160);
    global.__GR_NEST_COMPARE__ = acc;
  }

  function enqueueRpaFlush(ctx, source, fields, force) {
    if (!fields || typeof fields !== "object") return;
    fields.rpa_source = source;
    if (!fields.page_url) fields.page_url = pageUrlOf(ctx);
    if (fields.page_id == null) fields.page_id = pageIdOf(ctx);
    enqueue(ctx.queue, {
      session_id: ctx.session_id,
      batch_id: "B11_interaction",
      source: source,
      inject_path: ctx.inject_path,
      priority: fields.pagehide_flush ? 100 : 88,
      force: !!force || !!fields.pagehide_flush,
      payload: {
        fields: fields,
        sandbox_kind: fields.sandbox_kind || source.split(":")[0] || "iframe",
      },
    });
  }

  function runWorker(ctx, timeoutMs) {
    return new Promise(function (resolve) {
      var t0 = Date.now();
      if (typeof Worker === "undefined") {
        resolve({ kind: "worker", received: [], blocked: true, partial: true, ms: 0, rpa: false });
        return;
      }
      // Worker bootstrap: identity snapshot + continuous RPA (timer/message activity).
      // Workers cannot observe pointer/keyboard — honest worker_tick/worker_message monitors.
      // Worker: identity + lite silicon (OffscreenCanvas residual + OfflineAudio when available).
      var code =
        "var SRC='worker:d1';" +
        "var events=[];var lastUp=0;var lastEv=0;var CAP=80;" +
        "function push(k,extra){if(events.length>=CAP)events.shift();var r={t:Date.now(),kind:k,type:k,source:SRC};" +
        "if(extra){for(var i in extra)r[i]=extra[i];}events.push(r);lastEv=Date.now();}" +
        "function flush(reason){var fields={behavior_early_bound:true,behavior_events:events.slice(),behavior_count:events.length," +
        "pagehide_flush:reason==='close',rpa_flush_reason:reason||'tick',rpa_idle_flush:reason==='idle_30s'," +
        "collected_at:Date.now(),rpa_source:SRC,sandbox_kind:'worker',worker_rpa:true};" +
        "lastUp=Date.now();self.postMessage({from:'gr_worker_rpa',type:'rpa_flush',source:SRC,ok:true,payload:fields});}" +
        "function siliconLite(base){" +
        "var o={};for(var k in base)o[k]=base[k];o.nest_silicon_algo='gr_nest_silicon_v3b_v1';" +
        // Worker GL governor: single live OffscreenCanvas WebGL context.
        "try{if(typeof OffscreenCanvas!=='undefined'&&OffscreenCanvas.prototype&&!OffscreenCanvas.prototype.__gr_gl_gov__){" +
        "var _og=OffscreenCanvas.prototype.getContext;OffscreenCanvas.prototype.getContext=function(t,a){" +
        "var tl=String(t||'').toLowerCase();if(tl.indexOf('webgl')>=0){try{if(self.__gr_last_gl&&self.__gr_last_gl.useProgram)self.__gr_last_gl.useProgram(null);}catch(e){}var g=_og.apply(this,arguments);self.__gr_last_gl=g;return g;}" +
        "return _og.apply(this,arguments);};OffscreenCanvas.prototype.__gr_gl_gov__=1;}}catch(eGov){}" +
        // Same commercial residual shader+strip mean as main (not hist-lite ~0.06 scale).
        "try{if(typeof OffscreenCanvas!=='undefined'){var SZ=64;var c=new OffscreenCanvas(SZ,SZ);" +
        "var gl=c.getContext('webgl',{antialias:false,preserveDrawingBuffer:true})||c.getContext('experimental-webgl',{antialias:false,preserveDrawingBuffer:true});" +
        "if(gl){var vs=gl.createShader(gl.VERTEX_SHADER);gl.shaderSource(vs,'attribute vec2 a;void main(){gl_Position=vec4(a,0.0,1.0);}');gl.compileShader(vs);" +
        "var fs=gl.createShader(gl.FRAGMENT_SHADER);gl.shaderSource(fs,'precision mediump float;void main(){vec2 uv=gl_FragCoord.xy;float k=0.0;float ax=uv.x*(0.137+k*0.01)+uv.y*(0.091+k*0.007);float ay=uv.y*(0.173+k*0.009)-uv.x*(0.053+k*0.005);float n=sin(ax*1.7)*cos(ay*2.3)+sin((ax+ay)*(3.1+k));float m=fract(pow(abs(n)+1.001,1.61)*(97.3+k*3.0));float t=fract(m*17.0+k+uv.x*uv.y*0.00031);gl_FragColor=vec4(t,fract(t*3.7+m*2.1),fract(m+t),1.0);}');gl.compileShader(fs);" +
        "var prog=gl.createProgram();gl.attachShader(prog,vs);gl.attachShader(prog,fs);gl.linkProgram(prog);" +
        "if(gl.getProgramParameter(prog,gl.LINK_STATUS)){gl.useProgram(prog);var buf=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buf);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-1,-1,3,-1,-1,3]),gl.STATIC_DRAW);" +
        "var loc=gl.getAttribLocation(prog,'a');gl.enableVertexAttribArray(loc);gl.vertexAttribPointer(loc,2,gl.FLOAT,false,0,0);gl.viewport(0,0,SZ,SZ);gl.clearColor(0,0,0,1);gl.clear(gl.COLOR_BUFFER_BIT);gl.drawArrays(gl.TRIANGLES,0,3);gl.drawArrays(gl.TRIANGLES,0,3);" +
        "var pix=new Uint8Array(SZ*SZ*4);gl.readPixels(0,0,SZ,SZ,gl.RGBA,gl.UNSIGNED_BYTE,pix);" +
        "var strips=6,stripH=Math.floor(SZ/strips)||1,curve=[],gSum=0,gSum2=0,gN=0,ti,y0,y1,y,x,off,v,sum,sum2,nn,mean,varr;" +
        "for(ti=0;ti<strips;ti++){y0=ti*stripH;y1=ti===strips-1?SZ:y0+stripH;sum=0;sum2=0;nn=0;" +
        "for(y=y0;y<y1;y++){for(x=0;x<SZ;x+=2){off=(y*SZ+x)*4;v=(pix[off]+pix[off+1]*0.35)/(255*1.35);sum+=v;sum2+=v*v;nn++;gSum+=v;gSum2+=v*v;gN++;}}" +
        "if(nn<1){curve.push(0);continue;}mean=sum/nn;varr=sum2/nn-mean*mean;if(varr<0)varr=0;curve.push(Math.round(Math.sqrt(varr)*1e5)/1e5);}" +
        "if(gN>0){var gm=gSum/gN;var gv=gSum2/gN-gm*gm;if(gv<0)gv=0;curve.push(Math.round(gm*1e5)/1e5);curve.push(Math.round(Math.sqrt(gv)*1e5)/1e5);}" +
        "var cSum=0;for(ti=0;ti<curve.length;ti++)cSum+=curve[ti];o.nest_hw_curve_webgl=curve;o.nest_residual_mean=Math.round((cSum/(curve.length||1))*1e6)/1e6;" +
        "var cVa=0;for(ti=0;ti<curve.length;ti++){var dd=curve[ti]-o.nest_residual_mean;cVa+=dd*dd;}o.nest_residual_std=Math.round(Math.sqrt(cVa/(curve.length||1))*1e6)/1e6;" +
        "o.nest_residual_ok=o.nest_residual_std>0;o.nest_residual_algo='gr_webgl_residual_hist_v3b';o.nest_webgl_probe_path='worker_noderiv_v3b';" +
        "try{if(gl.useProgram)gl.useProgram(null);}catch(eL){}" +
        "}else{o.nest_webgl_probe_path='link_fail';}" +
        "}else{o.nest_webgl_probe_path='no_gl';}}else{o.nest_webgl_probe_path='no_offscreen';}}catch(eG){o.nest_webgl_probe_path='err:'+String(eG&&eG.message||eG);}" +
        "try{var OAC=self.OfflineAudioContext||self.webkitOfflineAudioContext;" +
        "if(OAC){var ctx=new OAC(1,1024,44100);var osc=ctx.createOscillator();osc.type='triangle';osc.frequency.value=10000;" +
        "osc.connect(ctx.destination);osc.start(0);" +
        "return ctx.startRendering().then(function(buf){try{var data=buf.getChannelData(0);var binsA=[],i,j,s2,nb=8,st=Math.floor(data.length/nb);" +
        "for(i=0;i<nb;i++){s2=0;for(j=0;j<st;j++){var v=data[i*st+j]||0;s2+=v*v;}binsA.push(Math.round(Math.sqrt(s2/(st||1))*1e6)/1e6);}" +
        "o.nest_hw_curve_audio=binsA;o.nest_audio_noise_sr=44100;o.nest_audio_noise_algo='gr_nest_audio_oac_lite_v1';o.nest_audio_probe_path='worker_oac_lite';" +
        "}catch(eB){o.nest_audio_probe_path='oac_parse_err';}o.nest_silicon_ready=true;return o;}).catch(function(){" +
        "o.audio_sample_rate=44100;o.nest_audio_probe_path='oac_fail';o.nest_silicon_ready=true;return o;});}" +
        "else{o.nest_audio_probe_path='no_oac';o.nest_silicon_ready=true;return Promise.resolve(o);}" +
        "}catch(eA){o.nest_audio_probe_path='err';o.nest_silicon_ready=true;return Promise.resolve(o);}}" +
        "self.onmessage=function(ev){push('worker_message',{data_type:typeof (ev&&ev.data)});" +
        "if(ev&&ev.data&&ev.data==='go'){try{var nav=self.navigator||{};" +
        "var tz='';try{tz=Intl.DateTimeFormat().resolvedOptions().timeZone||'';}catch(e){}" +
        "var off=null;try{off=new Date().getTimezoneOffset();}catch(e2){}" +
        "var p=(nav.platform||'').toLowerCase();var u=(nav.userAgent||'').toLowerCase();" +
        "var osf='other';if(/win/.test(p)||/windows/.test(u))osf='windows';" +
        "else if(/mac/.test(p)||/mac os/.test(u)||/iphone|ipad|ios/.test(u))osf=/iphone|ipad|ios/.test(u)?'ios':'macos';" +
        "else if(/android/.test(u)||/android/.test(p))osf='android';" +
        "else if(/linux|x11/.test(p)||/linux/.test(u))osf='linux';" +
        "var langs=[];try{langs=nav.languages?Array.prototype.slice.call(nav.languages,0,8):[];}catch(eL){}" +
        "var toStrN=false;try{toStrN=Function.prototype.toString.toString().indexOf('[native code]')>=0;}catch(eT){}" +
        "var base={" +
        "user_agent:nav.userAgent||'',hardware_concurrency:nav.hardwareConcurrency!=null?nav.hardwareConcurrency:null," +
        "platform:nav.platform||'',os_family:osf,language:nav.language||'',languages:langs,webdriver:!!nav.webdriver," +
        "device_memory:nav.deviceMemory!=null?nav.deviceMemory:null,timezone:tz,timezone_offset_min:off," +
        "vendor:nav.vendor||'',max_touch_points:nav.maxTouchPoints!=null?nav.maxTouchPoints:null," +
        "toStringNative:toStrN,consistency_probe:true,identity_surface:true,rpa_surface_ready:true," +
        "sandbox_kind:'worker'" +
        "};" +
        "self.postMessage({from:'gr_worker',ok:true,source:SRC,payload:base});" +
        "siliconLite(base).then(function(full){self.postMessage({from:'gr_worker',ok:true,source:SRC,payload:full,silicon_recollect:true});});" +
        "}catch(e){self.postMessage({from:'gr_worker',ok:false,error:String(e)});}" +
        "flush('bind');}};" +
        // 5s continuous / 2.5s tick — was 2s/1.5s and flooded gecko B11 (broken-pipe + soft_exhausted).
        "var n=0;setInterval(function(){n++;push('worker_tick',{n:n});var now=Date.now();" +
        "if(events.length&&now-lastUp>=5000)flush('continuous');" +
        "else if(lastUp&&now-lastUp>=30000&&now-lastEv>=30000)flush('idle_30s');},2500);";
      var blob = new Blob([code], { type: "application/javascript" });
      var url = URL.createObjectURL(blob);
      var w = new Worker(url);
      var done = false;
      var received = [];
      var rpaFlushes = 0;
      function finish(r) {
        if (done) return;
        done = true;
        // Keep worker alive for continuous RPA (do not terminate immediately).
        liveNests.push({ kind: "worker", worker: w, url: url });
        resolve(r);
      }
      var timer = setTimeout(function () {
        finish({
          kind: "worker",
          received: received.slice(),
          partial: received.length === 0,
          blocked: false,
          ms: Date.now() - t0,
          rpa: rpaFlushes > 0,
        });
      }, timeoutMs || 4000);
      w.onmessage = function (ev) {
        var d = ev.data || {};
        if (d.from === "gr_worker_rpa" && d.type === "rpa_flush" && d.payload) {
          rpaFlushes++;
          enqueueRpaFlush(ctx, d.source || "worker:d1", d.payload, true);
          return;
        }
        if (d.from === "gr_worker" && d.ok && d.payload) {
          var isSil = !!d.silicon_recollect;
          if (received.indexOf("worker:d1") < 0) received.push("worker:d1");
          try {
            if (!d.payload.nest_engine_family) d.payload.nest_engine_family = "worker";
            noteNestCompare(d.payload);
          } catch (eN) {}
          enqueue(ctx.queue, {
            session_id: ctx.session_id,
            batch_id: "B7_sandbox",
            source: "worker:d1",
            inject_path: ctx.inject_path,
            priority: isSil ? 42 : 40,
            payload: {
              fields: d.payload,
              sandbox_kind: "worker",
              silicon_recollect: isSil,
            },
          });
          if (!isSil) {
            clearTimeout(timer);
            finish({
              kind: "worker",
              received: received.slice(),
              partial: false,
              blocked: false,
              ms: Date.now() - t0,
              rpa: rpaFlushes > 0,
              last_fields: d.payload,
            });
          } else {
            try {
              // Keep silicon fields for summary compare even after identity finish.
              global.__GR_NEST_COMPARE__ = global.__GR_NEST_COMPARE__ || {};
              global.__GR_NEST_COMPARE__.silicon_fields = d.payload;
            } catch (eS) {}
          }
        }
      };
      w.onerror = function () {
        clearTimeout(timer);
        finish({
          kind: "worker",
          received: [],
          blocked: true,
          partial: true,
          ms: Date.now() - t0,
          rpa: false,
        });
      };
      w.postMessage("go");
    });
  }

  function runIframe(ctx, kind, timeoutMs) {
    return new Promise(function (resolve) {
      var t0 = Date.now();
      var iframe = document.createElement("iframe");
      iframe.setAttribute("aria-hidden", "true");
      // Visible hit target size 0 but keep in DOM for RPA — pointer-events none on chrome,
      // child still receives synthetic/real events when interacted; bind is always live.
      iframe.style.cssText =
        "width:2px;height:2px;border:0;position:fixed;left:0;top:0;opacity:0.01;pointer-events:auto;z-index:-1";
      if (kind === "sandbox_iframe") {
        try {
          // allow-same-origin needed for some browsers to postMessage + run scripts with RPA
          iframe.setAttribute("sandbox", "allow-scripts allow-same-origin");
        } catch (e) {}
      }
      var settled = false;
      var received = [];
      var rpaFlushes = 0;

      function finish(partial) {
        if (settled) return;
        settled = true;
        // Keep iframe alive for continuous RPA flushes — do NOT remove.
        liveNests.push({ kind: kind, iframe: iframe });
        resolve({
          kind: kind,
          received: received.slice(),
          partial: !!partial,
          blocked: received.length === 0,
          ms: Date.now() - t0,
          rpa: rpaFlushes > 0,
        });
      }

      var timer = setTimeout(function () {
        finish(true);
      }, timeoutMs || 4500);

      function onMsg(ev) {
        var d = ev.data;
        if (!d || d.from !== "gr_nest") return;
        // Continuous RPA flush from nest_frame (real child events)
        if (d.type === "rpa_flush" && d.payload) {
          rpaFlushes++;
          enqueueRpaFlush(ctx, d.source || kind + ":d1", d.payload, !!d.force);
          return;
        }
        if (d.type !== "result" && d.type != null && d.type !== "result") return;
        // Identity snapshot (+ optional silicon recollect for multi-source residual)
        if (d.ok === false) return;
        var source = d.source || kind + ":d1";
        var isSilicon = !!d.silicon_recollect;
        if (received.indexOf(source) >= 0 && !isSilicon) return;
        if (received.indexOf(source) < 0) received.push(source);
        try {
          var pl = d.payload || {};
          if (!pl.nest_engine_family) pl.nest_engine_family = kind;
          noteNestCompare(pl);
        } catch (eNc) {}
        enqueue(ctx.queue, {
          session_id: ctx.session_id,
          batch_id: "B7_sandbox",
          source: source,
          inject_path: ctx.inject_path,
          priority: isSilicon ? 42 : 40,
          payload: {
            fields: d.payload || {},
            sandbox_kind: kind,
            silicon_recollect: isSilicon,
          },
        });
        // First identity result finishes tree timing; silicon may arrive after.
        if (!isSilicon) {
          clearTimeout(timer);
          finish(false);
        }
      }

      // Persistent message listener for RPA (not removed after first result)
      window.addEventListener("message", onMsg);
      liveNests.push({ kind: kind + "_listener", onMsg: onMsg });

      // PV first-party SDK only (/g5/dist/...). Standard C: opaque nest_frame only.
      function manifestAsset(key) {
        try {
          var man =
            global.__GR_MANIFEST__ ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
            null;
          if (man && man.assets && man.assets[key]) return String(man.assets[key]);
          if (key === "gl_governor" && man && man.gl_governor_url)
            return String(man.gl_governor_url);
        } catch (eM) {}
        return "";
      }
      function isOpaqueNestUrl(s) {
        try {
          var base = String(s || "")
            .split("?")[0]
            .split("#")[0]
            .replace(/^.*\//, "");
          // pure opaque nest: <hash>.html  OR legacy hashed nest_frame.<hash>.html
          return (
            /^[a-f0-9]{8,16}\.html$/i.test(base) ||
            /^nest_frame\.[a-f0-9]{8,16}\.html$/i.test(base)
          );
        } catch (e) {
          return false;
        }
      }
      function resolveNestBase(raw) {
        var fallback = manifestAsset("nest_frame") || "";
        var s = String(raw || "").trim();
        if (s && isOpaqueNestUrl(s)) return s.split("?")[0];
        if (/^https?:\/\//i.test(s)) {
          try {
            var u = new URL(s);
            if (isOpaqueNestUrl(u.pathname)) return s.split("?")[0];
          } catch (eU) {}
        }
        // Opaque-only: never invent nest_frame.html / nest_frame.<gen>.html on the wire.
        return fallback;
      }
      var nestBase = resolveNestBase(ctx && ctx.nestUrl);
      if (!nestBase) {
        // Cannot open nest without opaque URL — skip quietly.
        clearTimeout(timer);
        finish(false);
        return;
      }
      var glUrl = manifestAsset("gl_governor");
      var rpaUrl = manifestAsset("rpa_monitor");
      var nest =
        nestBase +
        "?kind=" +
        encodeURIComponent(kind) +
        "&session_id=" +
        encodeURIComponent(ctx.session_id || "") +
        "&page_id=" +
        encodeURIComponent(pageIdOf(ctx) || "") +
        "&page_url=" +
        encodeURIComponent(pageUrlOf(ctx)) +
        "&parent_href=" +
        encodeURIComponent(pageUrlOf(ctx)) +
        "&inject_path=" +
        encodeURIComponent(ctx.inject_path || "");
      if (glUrl) nest += "&gl=" + encodeURIComponent(glUrl);
      if (rpaUrl) nest += "&rpa=" + encodeURIComponent(rpaUrl);
      iframe.src = nest;
      iframe.onerror = function () {
        clearTimeout(timer);
        finish(true);
      };
      (document.body || document.documentElement).appendChild(iframe);
    });
  }

  async function runTree(ctx, caps) {
    caps = caps || {};
    // Brain sandbox_plan: product requires ≥2 nest kinds in one B7 batch.
    var plan = (ctx && ctx.sandbox_plan) || (caps && caps.sandbox_plan) || global.__GR_SANDBOX_PLAN__ || null;
    var kinds;
    if (plan && plan.kinds && plan.kinds.length) {
      kinds = plan.kinds.slice();
    } else {
      kinds = ["iframe", "worker"];
    }
    // Enforce min 2 kinds even if brain sent a single kind (legacy plans)
    if (kinds.length < 2) {
      if (kinds.indexOf("iframe") < 0) kinds.push("iframe");
      if (kinds.indexOf("worker") < 0) kinds.push("worker");
      if (kinds.length < 2) kinds.push("sandbox_iframe");
    }
    var maxConc =
      (plan && plan.max_concurrent) ||
      (caps.maxConcurrent != null ? caps.maxConcurrent : kinds.length);
    // Default stagger: 1 concurrent nest (avoid dual GPU/silicon lite with B10/B10x).
    if (maxConc == null || !(maxConc > 0)) maxConc = 1;
    maxConc = Math.max(1, Math.min(3, 0 | maxConc));
    var timeoutMs = caps.short_visit ? 3500 : 5000;
    var results = [];
    // Nest surfaces share ResourceBus with main: wait only for classes we will use.
    // Default nest silicon does GPU+audio — wait those free, then run under locks.
    async function withNestResources(sourceTag, fn) {
      var PL = global.GRPackLoader;
      if (PL && typeof PL.withResourceLock === "function") {
        // Nest takes nest class first (orchestrator), then gpu then audio serially inside.
        return PL.withResourceLock("nest", "B7:" + sourceTag, function () {
          return PL.withResourceLock("gpu", "nest_gpu:" + sourceTag, function () {
            return PL.withResourceLock("audio", "nest_audio:" + sourceTag, function () {
              return Promise.resolve().then(fn);
            }, sourceTag);
          }, sourceTag);
        }, sourceTag);
      }
      // Fallback: legacy HW lock wait
      var waitN = 0;
      while (global.__GR_HW_LOCK__ && waitN < 80) {
        await new Promise(function (r) {
          setTimeout(r, 50);
        });
        waitN++;
      }
      return fn();
    }
    if (maxConc <= 1) {
      // Serial nest kinds — ResourceBus releases between kinds.
      for (var ki = 0; ki < kinds.length; ki++) {
        var kk = kinds[ki];
        var srcTag = kk === "worker" ? "worker:d1" : kk + ":d1";
        var one = await withNestResources(srcTag, function () {
          return kk === "worker" ? runWorker(ctx, timeoutMs) : runIframe(ctx, kk, timeoutMs);
        });
        results.push(one);
      }
    } else {
      // maxConc>1: ResourceBus still serializes gpu/audio; kinds await in order.
      for (var kj = 0; kj < kinds.length; kj++) {
        var kk2 = kinds[kj];
        var src2 = kk2 === "worker" ? "worker:d1" : String(kk2) + ":d1";
        var kindSnap = kk2;
        results.push(
          await withNestResources(src2, function () {
            return kindSnap === "worker"
              ? runWorker(ctx, timeoutMs)
              : runIframe(ctx, kindSnap, timeoutMs);
          })
        );
      }
    }
    var all = [];
    results.forEach(function (r) {
      (r.received || []).forEach(function (s) {
        if (all.indexOf(s) < 0) all.push(s);
      });
    });
    var blockedCount = 0;
    var partialCount = 0;
    results.forEach(function (r) {
      if (r && r.blocked) blockedCount++;
      if (r && r.partial) partialCount++;
    });
    // JS ran (this function) but all nest surfaces blocked → browser/sandbox integrity issue
    var sandbox_blocked = kinds.length > 0 && all.length === 0;
    var sandbox_partial = all.length > 0 && all.length < kinds.length;
    var sandbox_all_empty = sandbox_blocked || all.length === 0;
    var sandbox_under_two = all.length > 0 && all.length < 2;
    // Capability score (0..1) for analysis OS/BR/RPA demotion
    var capability = 0;
    if (sandbox_all_empty) capability = 0;
    else if (sandbox_under_two) capability = 0.45;
    else if (all.length >= 2 && !sandbox_partial) capability = 0.9;
    else if (all.length >= 2) capability = 0.7;
    else capability = 0.35;
    var band = capability <= 0.05 ? "dead" : capability < 0.55 ? "thin" : capability < 0.8 ? "partial" : "ok";
    // Structured nest vs main compare (fp_channel sandbox_consistency) — nest_* only
    var nestAcc = global.__GR_NEST_COMPARE__ || {};
    var nestResidualMean =
      nestAcc.nest_residual_mean != null ? nestAcc.nest_residual_mean : null;
    var nestEngineFamily = nestAcc.nest_engine_family || null;
    try {
      for (var ri = 0; ri < results.length; ri++) {
        var rr = results[ri] || {};
        if (nestResidualMean == null && rr.last_fields && rr.last_fields.nest_residual_mean != null) {
          nestResidualMean = rr.last_fields.nest_residual_mean;
        }
        if (!nestEngineFamily && rr.last_fields && rr.last_fields.nest_engine_family) {
          nestEngineFamily = rr.last_fields.nest_engine_family;
        }
      }
      if (nestAcc.silicon_fields && nestResidualMean == null) {
        nestResidualMean = nestAcc.silicon_fields.nest_residual_mean;
        if (!nestEngineFamily) {
          nestEngineFamily = nestAcc.silicon_fields.nest_engine_family || "worker";
        }
      }
    } catch (eNest) {}
    var mainResidual =
      global.__GR_MAIN_RESIDUAL_MEAN__ != null
        ? global.__GR_MAIN_RESIDUAL_MEAN__
        : null;
    // nest hist-lite (~0.06) vs commercial residual (~0.26) are not comparable.
    var nestVsMainAgree = null;
    var nestVsMainComparable = false;
    if (nestResidualMean != null && mainResidual != null) {
      var nm = Number(nestResidualMean);
      var mm = Number(mainResidual);
      nestVsMainComparable =
        (mm >= 0.15 && nm >= 0.15) || (mm < 0.12 && nm < 0.12);
      if (nestVsMainComparable) {
        nestVsMainAgree = Math.abs(nm - mm) < 0.001;
      }
    }
    enqueue(ctx.queue, {
      session_id: ctx.session_id,
      batch_id: "B7_sandbox",
      source: "main",
      inject_path: ctx.inject_path,
      priority: 35,
      payload: {
        fields: {
          schema: "gr.sandbox_tree_summary.v2",
          sources_received: all,
          sandbox_sources_received: all,
          kinds: kinds,
          sandbox_kinds_planned: kinds,
          sandbox_triggered: all.length > 0,
          sandbox_blocked: sandbox_blocked,
          sandbox_partial: sandbox_partial,
          sandbox_all_empty: sandbox_all_empty,
          sandbox_under_two_kinds: sandbox_under_two,
          sandbox_payload_source_n: all.length,
          sandbox_blocked_count: blockedCount,
          sandbox_partial_count: partialCount,
          sandbox_ok: all.length > 0 && !sandbox_blocked,
          sandbox_capability_score: capability,
          sandbox_capability_band: band,
          js_ok_sandbox_dead: sandbox_all_empty,
          rpa_multi_source: true,
          results: results,
          nest_residual_mean: nestResidualMean,
          nest_engine_family: nestEngineFamily,
          nest_vs_main_comparable: nestVsMainComparable,
          nest_vs_main_agree_0p001: nestVsMainAgree,
          nest_vs_main_note: nestVsMainComparable
            ? null
            : nestResidualMean != null && mainResidual != null
              ? "algo_scale_mismatch_hist_vs_residual"
              : null,
        },
        sandbox_kind: "main",
      },
    });
    global.__GR_SANDBOX_RESULT__ = {
      kinds: kinds,
      received: all,
      results: results,
      rpa_live: true,
      sandbox_blocked: sandbox_blocked,
      sandbox_partial: sandbox_partial,
      sandbox_all_empty: sandbox_all_empty,
      sandbox_capability_score: capability,
      sandbox_capability_band: band,
      nest_residual_mean: nestResidualMean,
      nest_engine_family: nestEngineFamily,
      nest_vs_main_comparable: nestVsMainComparable,
      nest_vs_main_agree_0p001: nestVsMainAgree,
    };
    return global.__GR_SANDBOX_RESULT__;
  }

  /** Test/helper: count live nest surfaces with RPA capability. */
  function liveSurfaceCount() {
    return liveNests.filter(function (n) {
      return n.iframe || n.worker;
    }).length;
  }

  global.GRSandbox = {
    runTree: runTree,
    liveSurfaceCount: liveSurfaceCount,
    _liveNests: liveNests,
  };
})(typeof window !== "undefined" ? window : globalThis);

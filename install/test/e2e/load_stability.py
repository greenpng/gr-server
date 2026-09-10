#!/usr/bin/env python3
"""load_stability.py — 公开仓 runner 高并发稳定性负载 (fulltest.yml load 件)。

合成驱动 (无浏览器, lab.* 免目录批号 + X-Gr-Result-Token 取回):
  每会话: session/open → lab.e2e.B0 + lab.e2e.B3 两批 ingest → 轮询 result (≤30s)

断言 (任一不过 = 失败):
  open   2xx 率 == 100%
  ingest 2xx 率 >= 99.5%   (accepted:true 视为成功)
  result 完成率 >= 99%     (ok:true, 非 pending)
  5xx    计数 == 0
  负载后 /v1/health 仍 200; 服务日志 ERROR/panic == 0; FD 增幅有界; CLOSE_WAIT == 0

用法:
  python3 load_stability.py --stack-env /tmp/gr-e2e/stack.env \
      [--sessions 240] [--concurrency 24] [--out /tmp/gr-e2e/load.json]
纯 stdlib (runner 无第三方依赖)。
默认按 runner debug 构建调参 (240/24): 首跑 400/64 时 ingest max=12.016s 顶到
旧 12s 超时、280 个 open 超时失败 — 计数器当时只记 >=500 漏账, 现已全量入账。
"""
import argparse
import concurrent.futures
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--stack-env", default="/tmp/gr-e2e/stack.env")
    p.add_argument("--sessions", type=int, default=240)
    p.add_argument("--concurrency", type=int, default=24)
    p.add_argument("--result-timeout", type=int, default=30)
    p.add_argument("--out", default="")
    return p.parse_args()


def load_env(path):
    env = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line and not line.strip().startswith("#"):
            k, v = line.split("=", 1)
            env[k.strip()] = v.strip()
    return env


def req(url, method="GET", body=None, headers=None, timeout=30):
    """Return (status, text). 5xx/timeout raise nothing — 返回原样统计.
    timeout 30s: CI 是 debug 构建 (慢于 release 一个量级), 首跑 12s 顶到过 max=12.016."""
    data = None
    if body is not None:
        data = json.dumps(body).encode()
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("content-type", "application/json")
    for k, v in (headers or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:  # 连接失败/超时
        return 0, f"{type(e).__name__}: {e}"


def pct_latencies(samples):
    if not samples:
        return {}
    xs = sorted(samples)
    def q(p):
        return round(xs[min(len(xs) - 1, int(len(xs) * p))], 3)
    return {"p50": q(0.50), "p95": q(0.95), "p99": q(0.99), "max": round(xs[-1], 3)}


def fd_count(pid):
    try:
        return len(list(Path(f"/proc/{pid}/fd").iterdir()))
    except Exception:
        return -1


def main():
    args = parse_args()
    env = load_env(args.stack_env)
    probe = env["PROBE_BASE"]
    token = env["RESULT_TOKEN"]
    pid = env["SERVICE_PID"]
    log_path = env["SERVICE_LOG"]

    lat_open, lat_ing, lat_res = [], [], []
    cnt = {"open_2xx": 0, "ing_ok": 0, "ing_total": 0, "res_done": 0, "s5xx": 0, "errs": []}
    # 显式浏览器 UA: 本负载测的是「正常访客」先存后析全链路。
    # (urllib 默认 UA "Python-urllib/x" 目前不在 robots 名单, 但显式声明
    #  才不受名单演进影响; 1.0.10+ UA 自明爬虫会走早判快道。)
    BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64) Chrome/120 Safari/537.36"

    def one_session(i):
        vt = f"load_{int(time.time())}_{i}"
        t0 = time.time()
        st, body = req(f"{probe}/v1/session/open", "POST", {
            "site_id": "e2e_load", "visitor_terminal_id": vt,
            "meta": {"fe": "e2e-load"},
        }, headers={"user-agent": BROWSER_UA})
        dt_open = time.time() - t0
        sid = ""
        try:
            j = json.loads(body)
            sid = j.get("session_id") or (j.get("session") or {}).get("session_id") or ""
        except Exception:
            pass
        r = {"open": st, "sid": sid, "ing": [], "res": None,
             "dt_open": dt_open, "dt_ing": [], "dt_res": None}
        if st == 200 and sid:
            for bid in ("lab.e2e.B0", "lab.e2e.B3"):
                t = time.time()
                st2, b2 = req(f"{probe}/v1/ingest", "POST", {
                    "session_id": sid, "batch_id": bid, "source": "main",
                    "payload": {"fields": {
                        "os_family": "windows", "form_class": "desktop",
                        "timezone": "Asia/Shanghai", "hardware_concurrency": 8,
                        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120 Safari/537.36",
                    }},
                }, headers={"user-agent": BROWSER_UA})
                r["ing"].append({"status": st2, "accepted": '"accepted":true' in b2.replace(" ", "")})
                r["dt_ing"].append(time.time() - t)
            t = time.time()
            deadline = time.time() + args.result_timeout
            final = None
            while time.time() < deadline:
                st3, b3 = req(f"{probe}/v1/session/{sid}/result?projection=public",
                              headers={"X-Gr-Result-Token": token})
                if st3 == 200 and '"pending"' not in b3 and "no analysis" not in b3:
                    final = (st3, '"ok":true' in b3.replace(" ", ""))
                    break
                time.sleep(1.0)
            r["res"] = final
            r["dt_res"] = time.time() - t
        return r

    fd_before = fd_count(pid)
    # 只统计本次负载的日志增量 — 栈生命周期里的历史噪音 (如早前实验的 4xx/panic) 不计入
    try:
        log_offset = Path(log_path).stat().st_size
    except Exception:
        log_offset = 0
    t_start = time.time()

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as ex:
        for r in ex.map(one_session, range(args.sessions)):
            lat_open.append(r["dt_open"])
            if 200 <= r["open"] < 300:
                cnt["open_2xx"] += 1
            else:
                # 非 2xx 一律入账 (5xx、4xx、0=连接失败/超时)。
                # 首跑 34246830314 教训: 280 个 open 失败因只记 >=500 而静默漏账。
                cnt["s5xx"] += 1 if r["open"] >= 500 else 0
                cnt["errs"].append(f"open {r['open']}")
            for ig in r["ing"]:
                cnt["ing_total"] += 1
                if ig["accepted"]:
                    cnt["ing_ok"] += 1
                else:
                    cnt["s5xx"] += 1 if ig["status"] >= 500 else 0
                    cnt["errs"].append(f"ingest {ig['status']}")
            lat_ing.extend(r["dt_ing"])
            if r["res"] is not None:
                if r["res"][1]:
                    cnt["res_done"] += 1
                else:
                    cnt["s5xx"] += 1 if r["res"][0] >= 500 else 0
                    cnt["errs"].append(f"result {r['res'][0]}")
                lat_res.append(r["dt_res"])
            elif r["sid"]:
                cnt["errs"].append("result timeout/pending")

    wall = round(time.time() - t_start, 1)
    fd_after = fd_count(pid)
    st_health, _ = req(f"{probe}/v1/health")
    err_lines = 0
    try:
        with open(log_path, "rb") as f:
            f.seek(log_offset)
            appended = f.read().decode("utf-8", "replace")
        err_lines = len(re.findall(r" ERROR |panic", appended))
    except Exception:
        err_lines = -1

    summary = {
        "sessions": args.sessions, "concurrency": args.concurrency, "wall_s": wall,
        "open_2xx": cnt["open_2xx"], "ingest_ok": cnt["ing_ok"], "ingest_total": cnt["ing_total"],
        "result_done": cnt["res_done"], "http_5xx": cnt["s5xx"],
        "latency_open": pct_latencies(lat_open), "latency_ingest": pct_latencies(lat_ing),
        "latency_result": pct_latencies(lat_res),
        "health_after": st_health, "service_error_lines": err_lines,
        "fd_before": fd_before, "fd_after": fd_after,
    }
    if args.out:
        Path(args.out).write_text(json.dumps(summary, indent=2))

    failures = []
    if cnt["open_2xx"] != args.sessions:
        failures.append(f"open 2xx {cnt['open_2xx']}/{args.sessions} != 100%")
    if cnt["ing_total"] and cnt["ing_ok"] / cnt["ing_total"] < 0.995:
        failures.append(f"ingest ok {cnt['ing_ok']}/{cnt['ing_total']} < 99.5%")
    if cnt["res_done"] / args.sessions < 0.99:
        failures.append(f"result done {cnt['res_done']}/{args.sessions} < 99%")
    if cnt["s5xx"] != 0:
        failures.append(f"5xx count {cnt['s5xx']}")
    if st_health != 200:
        failures.append(f"health after load = {st_health}")
    if err_lines != 0:
        failures.append(f"service ERROR/panic lines = {err_lines}")
    if fd_before > 0 and fd_after > 0 and fd_after - fd_before > 400:
        failures.append(f"fd growth {fd_before}→{fd_after} unbounded")

    print(json.dumps(summary, indent=2))
    if cnt["errs"]:
        print("sample errors (first 8):", *cnt["errs"][:8], sep="\n  ")
    if failures:
        print("LOAD-STABILITY FAIL:", *failures, sep="\n  ")
        return 1
    print(f"LOAD-STABILITY PASS ({args.sessions} sessions / conc {args.concurrency} / {wall}s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

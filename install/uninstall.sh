#!/usr/bin/env bash
# greenpng 官方卸载脚本 (iss/audit OPS-02).
#
# 回收 install.sh / update_runtime_from_github.sh 建立的全部系统级常驻:
#   1. systemd: greenpng.service + greenpng-auto-upgrade.{service,timer}
#   2. polkit 自 OTA 提权规则 49-greenpng-self-ota.rules
#   3. 程序树 $INSTALL_ROOT (bin/fe/spec/modules/dist/data/log, VERSION)
#   4. 运行期锁 /run/greenpng-auto-upgrade.lock 与 OTA 临时缓存
#   5. --purge: 系统账户 greenpng 一并删除 (数据/日志随 $INSTALL_ROOT 整树移除)
#
# 用法:
#   bash uninstall.sh [--prefix /opt/greenpng] [--purge]
#
# 默认保守: 只删产品自身产物, 不碰数据库容器/卷 (docker compose 由
# install.sh --with-docker 建立的, 请用 `docker compose -f ... down` 单独处理)。
set -euo pipefail

PREFIX="${GR_PREFIX:-/opt/greenpng}"
PURGE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix) PREFIX="${2:-}"; shift 2 ;;
    --purge) PURGE=1; shift ;;
    -h|--help) sed -n '1,16p' "$0"; exit 0 ;;
    *) echo "[uninstall] unknown arg: $1" >&2; exit 2 ;;
  esac
done

say() { printf '[uninstall] %s\n' "$*"; }

if [[ $EUID -ne 0 ]]; then
  echo "[uninstall] ERROR: must run as root (systemd/polkit/user cleanup)" >&2
  exit 1
fi

# 1. 停止并注销 systemd 服务
say "stopping + disabling systemd units..."
for unit in greenpng.service greenpng-auto-upgrade.timer greenpng-auto-upgrade.service; do
  if systemctl cat "$unit" >/dev/null 2>&1 || systemctl list-unit-files "$unit" >/dev/null 2>&1; then
    systemctl stop "$unit" 2>/dev/null || true
    systemctl disable "$unit" 2>/dev/null || true
  fi
  rm -f "/etc/systemd/system/$unit"
done
systemctl daemon-reload || true
systemctl reset-failed 2>/dev/null || true

# 2. 清理 polkit 自 OTA 提权规则 (安全敏感: 必须移除)
say "removing polkit self-OTA rule..."
rm -f /etc/polkit-1/rules.d/49-greenpng-self-ota.rules

# 3. 清理程序安装树 (bin/fe/spec/modules/dist/data/log + VERSION 全在内)
if [[ -d "$PREFIX" ]]; then
  say "removing install tree: $PREFIX"
  rm -rf "$PREFIX"
else
  say "install tree absent: $PREFIX (nothing to remove)"
fi

# 4. 运行期锁与 OTA 临时缓存
say "removing runtime locks and OTA caches..."
rm -f /run/greenpng-auto-upgrade.lock
rm -rf /tmp/gr-ota-bundle /tmp/gr-runtime-ota-*.json 2>/dev/null || true

# 5. --purge: 系统账户 (数据与日志已随 $PREFIX 整树移除)
if [[ "$PURGE" -eq 1 ]]; then
  say "deep clean (--purge): removing system account greenpng"
  if id -u greenpng >/dev/null 2>&1; then
    userdel greenpng 2>/dev/null || true
  fi
else
  say "kept: system account 'greenpng' (use --purge to remove)"
fi

say "greenpng uninstall complete."
say "note: docker data services (postgres/redis) are NOT touched —"
say "      stop them separately if they were provisioned via --with-docker."

<template>
  <div class="page-stack" data-testid="view-dashboard">
    <div class="kpi-grid" data-testid="dash-cards">
      <div class="kpi-card accent">
        <div class="label">{{ t('dashboard.sessions') }}</div>
        <div class="value">{{ fmtInt(totals.sessions) }}</div>
        <div class="hint">{{ hours }}h · {{ t('results.summary') }}</div>
      </div>
      <div class="kpi-card">
        <div class="label">{{ t('dashboard.main_complete') }}</div>
        <div class="value">{{ fmtInt(totals.main_complete) }}</div>
        <div class="hint">{{ pct(totals.main_rate) }} {{ t('results.main_rate') }}</div>
      </div>
      <div class="kpi-card">
        <div class="label">{{ t('dashboard.sites') }}</div>
        <div class="value">{{ sitesCount }}</div>
        <div class="hint">{{ t('nav.sites') }}</div>
      </div>
      <div class="kpi-card">
        <div class="label">{{ t('dashboard.probe_status') }}</div>
        <div class="value" style="font-size:22px;margin-top:12px">
          <span class="status-dot" :class="probeOk ? 'ok' : 'bad'" />
          {{ probeOk ? t('app.probe_ok') : t('app.probe_bad') }}
        </div>
        <div class="hint">CPU {{ sys.cpu }} · Mem {{ sys.mem }}</div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">
        <span>{{ t('dashboard.product_kpis') }}</span>
        <div class="toolbar">
          <el-select v-model="hours" size="small" style="width:100px" data-testid="dash-range" @change="loadProduct">
            <el-option v-for="h in [1, 6, 24, 72, 168]" :key="h" :label="`${h}h`" :value="h" />
          </el-select>
          <el-button size="small" type="primary" plain @click="$router.push('/results')">{{ t('dashboard.view_results') }}</el-button>
        </div>
      </div>
      <div class="surface-body">
        <div v-if="rows.length" class="table-scroll">
        <el-table :data="rows" stripe size="small" data-testid="dash-completeness">
          <el-table-column prop="site_id" :label="t('sites.site_id')" width="120" />
          <el-table-column prop="product_version" :label="t('results.product_version')" width="120" />
          <el-table-column prop="sessions" :label="t('results.sessions_n')" width="100" />
          <el-table-column prop="main_complete" :label="t('results.main_complete')" width="120" />
          <el-table-column :label="t('results.main_rate')" width="120">
            <template #default="{ row }">
              <el-progress
                :percentage="ratePct(row.main_complete_rate)"
                :stroke-width="8"
                :show-text="true"
                style="max-width:140px"
              />
            </template>
          </el-table-column>
          <el-table-column prop="gateway_only" :label="t('results.gateway_only')" width="110" />
        </el-table>
        </div>
        <div v-else class="empty-state">{{ productErr || t('dashboard.no_data') }}</div>
      </div>
    </div>

    <div class="kpi-grid split-2">
      <div class="surface" data-testid="dash-action-dist">
        <div class="surface-header col">
          <span>{{ t('dashboard.action_dist') }}</span>
          <span class="hint-inline">{{ t('dashboard.action_dist_note') }}</span>
        </div>
        <div class="surface-body">
          <div v-if="actionBars.length" class="bar-list">
            <div v-for="b in actionBars" :key="b.key" class="bar-row">
              <div class="bar-label">
                <span>{{ b.key }}</span>
                <strong>{{ b.count }}</strong>
              </div>
              <div class="bar-track">
                <div class="bar-fill" :class="'tone-' + b.tone" :style="{ width: b.pct + '%' }" />
              </div>
            </div>
          </div>
          <div v-else class="empty-state">{{ actionErr || t('dashboard.no_data') }}</div>
          <div v-if="botBars.length" class="sub-bars">
            <div class="sub-title">{{ t('dashboard.bot_mix') }}</div>
            <div v-for="b in botBars" :key="'bot-' + b.key" class="bar-row compact">
              <div class="bar-label">
                <span>{{ b.key }}</span>
                <strong>{{ b.count }}</strong>
              </div>
              <div class="bar-track">
                <div class="bar-fill tone-muted" :style="{ width: b.pct + '%' }" />
              </div>
            </div>
          </div>
        </div>
      </div>
      <div class="surface">
        <div class="surface-header">{{ t('dashboard.system_health') }}</div>
        <div class="surface-body">
          <div class="sys-row"><span>{{ t('dashboard.cpu') }}</span><strong>{{ sys.cpu }}</strong></div>
          <div class="sys-row"><span>{{ t('dashboard.mem') }}</span><strong>{{ sys.mem }}</strong></div>
          <div class="sys-row"><span>{{ t('dashboard.load') }}</span><strong>{{ sys.load }}</strong></div>
          <div class="sys-row"><span>{{ t('dashboard.disk') }}</span><strong>{{ sys.disk }}</strong></div>
          <div class="sys-row"><span>{{ t('dashboard.cluster_nodes') }}</span><strong>{{ sys.nodes }}</strong></div>
        </div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('dashboard.quick_actions') }}</div>
      <div class="surface-body quick-actions">
        <el-button @click="$router.push('/sites')">{{ t('dashboard.view_sites') }}</el-button>
        <el-button @click="$router.push('/results')">{{ t('dashboard.view_results') }}</el-button>
        <el-button @click="$router.push('/strategies')">{{ t('nav.strategies') }}</el-button>
        <el-button @click="$router.push('/integrations')">{{ t('nav.integrations') }}</el-button>
        <el-button @click="$router.push('/retention')">{{ t('nav.retention') }}</el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'

const { t } = useI18n()
const tick = inject('refreshTick', ref(0))
const hours = ref(24)
const rows = ref([])
const productErr = ref('')
const actionErr = ref('')
const actionBars = ref([])
const botBars = ref([])
const sitesCount = ref(0)
const probeOk = ref(false)
const totals = reactive({ sessions: 0, main_complete: 0, main_rate: 0 })
const sys = reactive({ cpu: '—', mem: '—', load: '—', disk: '—', nodes: '—' })

function fmtInt(v) {
  const n = Number(v)
  return Number.isFinite(n) ? n.toLocaleString() : '—'
}
function pct(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return '—'
  return n <= 1 ? `${(n * 100).toFixed(1)}%` : `${n.toFixed(1)}%`
}
function ratePct(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return 0
  const p = n <= 1 ? n * 100 : n
  return Math.round(Math.max(0, Math.min(100, p)) * 10) / 10
}

function toBars(items, keyField, countField) {
  const list = (items || []).map((x) => ({
    key: String(x[keyField] ?? x.key ?? '—'),
    count: Number(x[countField] ?? x.count) || 0,
  }))
  const max = Math.max(1, ...list.map((x) => x.count))
  return list.map((x) => {
    let tone = 'muted'
    const k = x.key.toLowerCase()
    if (k === 'allow' || k.includes('human')) tone = 'ok'
    else if (k === 'deny' || k.includes('bot')) tone = 'bad'
    else if (k === 'challenge' || k.includes('suspect')) tone = 'warn'
    return { ...x, pct: Math.round((x.count / max) * 1000) / 10, tone }
  })
}

async function loadProduct() {
  productErr.value = ''
  actionErr.value = ''
  try {
    const j = await api(`results/summary?hours=${hours.value}`)
    rows.value = (j.by_site_version || []).slice(0, 12)
    let sessions = 0
    let main = 0
    for (const r of j.by_site_version || []) {
      sessions += Number(r.sessions) || 0
      main += Number(r.main_complete) || 0
    }
    totals.sessions = j.sessions ?? sessions
    totals.main_complete = j.main_complete ?? main
    totals.main_rate = j.main_complete_rate ?? (sessions ? main / sessions : 0)
    if (j.error) productErr.value = j.error
  } catch (e) {
    rows.value = []
    productErr.value = e.message || t('results.proxy_error')
  }
  try {
    const a = await api(`results/actions?hours=${hours.value}`)
    actionBars.value = toBars(a.by_action_proxy || [], 'action', 'count')
    botBars.value = toBars(a.by_bot_verdict || [], 'key', 'count').slice(0, 8)
    if (a.error) actionErr.value = a.error
  } catch (e) {
    actionBars.value = []
    botBars.value = []
    actionErr.value = e.message || t('results.proxy_error')
  }
}

async function loadSystem() {
  try {
    const j = await api(`dashboard?range=1h`)
    const m = j.node?.metrics || {}
    const disk = (m.disks || [])[0]
    sys.cpu = Number.isFinite(Number(m.cpu_pct)) ? `${Number(m.cpu_pct).toFixed(1)}%` : '—'
    sys.mem = Number.isFinite(Number(m.mem_pct)) ? `${Number(m.mem_pct).toFixed(1)}%` : '—'
    sys.load = Number.isFinite(Number(m.load_avg_one)) ? Number(m.load_avg_one).toFixed(2) : '—'
    sys.disk = disk && Number.isFinite(Number(disk.used_pct)) ? `${Number(disk.used_pct).toFixed(1)}%` : '—'
    sys.nodes = String((j.cluster?.nodes || []).length)
  } catch {
    /* ignore */
  }
  try {
    const s = await api('sites')
    sitesCount.value = (s.sites || []).length
  } catch {
    sitesCount.value = 0
  }
  try {
    const h = await api('probe/health')
    probeOk.value = !!h.ok
  } catch {
    probeOk.value = false
  }
}

async function load() {
  await Promise.all([loadProduct(), loadSystem()])
}

onMounted(load)
watch(tick, load)
</script>

<style scoped>
.sys-row {
  display: flex;
  justify-content: space-between;
  padding: 10px 0;
  border-bottom: 1px solid var(--gv-border);
  color: var(--gv-text-muted);
}
.sys-row:last-child { border-bottom: none; }
.sys-row strong { color: var(--gv-text); font-variant-numeric: tabular-nums; }
.quick-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.hint-inline {
  font-size: 11px;
  font-weight: 400;
  color: var(--gv-text-muted);
  display: block;
  margin-top: 4px;
  max-width: 100%;
  white-space: normal;
}
.surface-header.col {
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
}
.split-2 {
  grid-template-columns: 1fr 1fr;
}
@media (max-width: 900px) {
  .split-2 {
    grid-template-columns: 1fr;
  }
}
.bar-list { display: flex; flex-direction: column; gap: 10px; }
.bar-row .bar-label {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
  color: var(--gv-text-muted);
  margin-bottom: 4px;
}
.bar-row .bar-label strong {
  color: var(--gv-text);
  font-variant-numeric: tabular-nums;
}
.bar-track {
  height: 8px;
  border-radius: 999px;
  background: rgba(148, 163, 184, 0.15);
  overflow: hidden;
}
.bar-fill {
  height: 100%;
  border-radius: 999px;
  background: #6366f1;
  transition: width 0.25s ease;
}
.bar-fill.tone-ok { background: #22c55e; }
.bar-fill.tone-warn { background: #f59e0b; }
.bar-fill.tone-bad { background: #ef4444; }
.bar-fill.tone-muted { background: #64748b; }
.sub-bars { margin-top: 18px; padding-top: 12px; border-top: 1px solid var(--gv-border); }
.sub-title { font-size: 12px; color: var(--gv-text-muted); margin-bottom: 10px; }
.bar-row.compact { margin-bottom: 6px; }
</style>

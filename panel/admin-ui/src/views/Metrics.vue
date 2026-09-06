<template>
  <div class="page-stack" data-testid="view-metrics">
    <div class="kpi-grid">
      <div class="kpi-card"><div class="label">{{ t('dashboard.cpu') }}</div><div class="value">{{ cpu }}</div></div>
      <div class="kpi-card"><div class="label">{{ t('dashboard.mem') }}</div><div class="value">{{ mem }}</div></div>
      <div class="kpi-card"><div class="label">{{ t('dashboard.load') }}</div><div class="value">{{ loadAvg }}</div></div>
      <div class="kpi-card"><div class="label">{{ t('dashboard.disk') }}</div><div class="value">{{ disk }}</div></div>
    </div>
    <div class="surface">
      <div class="surface-header">{{ t('nav.metrics') }}</div>
      <div class="surface-body">
        <div v-if="rows.length" class="kv-list">
          <div v-for="r in rows" :key="r.k" class="sys-row">
            <span class="mono">{{ r.k }}</span>
            <strong>{{ r.v }}</strong>
          </div>
        </div>
        <pre v-else class="mono muted" data-testid="metrics-raw" style="margin:0;white-space:pre-wrap">{{ raw }}</pre>
      </div>
    </div>
  </div>
</template>
<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
const { t } = useI18n()
const raw = ref('')
const cpu = ref('—')
const mem = ref('—')
const loadAvg = ref('—')
const disk = ref('—')
const rows = ref([])
const tick = inject('refreshTick', ref(0))

function flatten(obj, prefix = '', acc = [], depth = 0) {
  if (!obj || typeof obj !== 'object' || depth > 3 || acc.length > 40) return acc
  for (const [k, v] of Object.entries(obj)) {
    if (acc.length > 40) break
    if (['cpu_pct', 'mem_pct', 'load_avg_one', 'disks'].includes(k) && !prefix) continue
    const key = prefix ? `${prefix}.${k}` : k
    if (v && typeof v === 'object' && !Array.isArray(v)) flatten(v, key, acc, depth + 1)
    else if (Array.isArray(v)) acc.push({ k: key, v: `${v.length}` })
    else acc.push({ k: key, v: v == null ? '—' : String(v) })
  }
  return acc
}

async function load() {
  const j = await api('metrics')
  raw.value = JSON.stringify(j, null, 2)
  cpu.value = fmt(j.cpu_pct) + '%'
  mem.value = fmt(j.mem_pct) + '%'
  loadAvg.value = fmt(j.load_avg_one, 2)
  const d = (j.disks || [])[0]
  disk.value = d ? fmt(d.used_pct) + '%' : '—'
  rows.value = flatten(j)
}
function fmt(v, d = 1) {
  const n = Number(v)
  return Number.isFinite(n) ? n.toFixed(d) : '—'
}
onMounted(load)
watch(tick, load)
</script>
<style scoped>
.kv-list { display: flex; flex-direction: column; }
.sys-row {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  padding: 10px 0;
  border-bottom: 1px solid var(--gv-border);
  color: var(--gv-text-muted);
  font-size: 13px;
}
.sys-row:last-child { border-bottom: none; }
.sys-row strong {
  color: var(--gv-text);
  font-weight: 500;
  text-align: right;
  word-break: break-word;
}
</style>

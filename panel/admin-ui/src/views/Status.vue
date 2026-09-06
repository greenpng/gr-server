<template>
  <div class="page-stack" data-testid="view-status">
    <div class="surface">
      <div class="surface-header">{{ t('nav.status') }}</div>
      <div class="surface-body">
        <div v-if="rows.length" class="kv-list">
          <div v-for="r in rows" :key="r.k" class="sys-row">
            <span class="mono">{{ r.k }}</span>
            <strong>{{ r.v }}</strong>
          </div>
        </div>
        <pre v-else class="mono muted" data-testid="status-raw" style="margin:0;white-space:pre-wrap">{{ raw || t('status.empty') }}</pre>
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
const rows = ref([])
const tick = inject('refreshTick', ref(0))

function flatten(obj, prefix = '', acc = [], depth = 0) {
  if (!obj || typeof obj !== 'object' || depth > 3 || acc.length > 48) return acc
  for (const [k, v] of Object.entries(obj)) {
    if (acc.length > 48) break
    const key = prefix ? `${prefix}.${k}` : k
    if (v && typeof v === 'object' && !Array.isArray(v)) flatten(v, key, acc, depth + 1)
    else if (Array.isArray(v)) acc.push({ k: key, v: `${v.length}` })
    else acc.push({ k: key, v: v == null ? '—' : String(v) })
  }
  return acc
}

async function load() {
  const j = await api('status')
  raw.value = JSON.stringify(j, null, 2)
  rows.value = flatten(j)
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
  max-width: 65%;
  word-break: break-word;
}
</style>

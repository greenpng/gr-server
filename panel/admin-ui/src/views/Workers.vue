<template>
  <div class="page-stack" data-testid="view-workers">
    <el-alert type="info" :closable="false" show-icon :title="t('workers.note')" />
    <div class="surface">
      <div class="surface-header">{{ t('nav.workers') }}</div>
      <div class="surface-body worker-grid">
        <div class="field">
          <label>{{ t('workers.analyze') }}</label>
          <el-input-number v-model="analyze" :min="1" :max="64" data-testid="w-analyze" />
          <div class="hint">{{ t('workers.analyze_hint') }}</div>
        </div>
        <div class="field">
          <label>{{ t('workers.ingest') }}</label>
          <el-input-number v-model="ingest" :min="1" :max="64" data-testid="w-ingest" />
          <div class="hint">{{ t('workers.ingest_hint') }}</div>
        </div>
        <div class="field">
          <label>{{ t('workers.gateway') }}</label>
          <el-input-number v-model="gateway" :min="1" :max="64" data-testid="w-gateway" />
          <div class="hint">{{ t('workers.gateway_hint') }}</div>
        </div>
        <div class="field field-action">
          <el-button type="primary" data-testid="w-save" @click="save">{{ t('app.save') }}</el-button>
        </div>
      </div>
      <div v-if="out" class="surface-body" style="padding-top:0">
        <pre class="mono muted" data-testid="w-out" style="margin:0">{{ out }}</pre>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage } from 'element-plus'

const { t } = useI18n()
const analyze = ref(1)
const ingest = ref(1)
const gateway = ref(1)
const out = ref('')
const tick = inject('refreshTick', ref(0))

async function load() {
  const j = await api('workers')
  analyze.value = j.analyze
  ingest.value = j.ingest
  gateway.value = j.gateway
}
async function save() {
  const j = await api('workers', {
    method: 'POST',
    body: JSON.stringify({ analyze: analyze.value, ingest: ingest.value, gateway: gateway.value }),
  })
  out.value = j.restart_required ? JSON.stringify(j, null, 2) : ''
  ElMessage.success(j.restart_required ? t('app.success') : t('workers.hot_ok'))
}
onMounted(load)
watch(tick, load)
</script>

<style scoped>
.worker-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 16px 20px;
  align-items: end;
}
.field label {
  display: block;
  font-size: 12px;
  color: var(--gv-text-muted);
  margin-bottom: 6px;
}
.field .hint {
  margin-top: 6px;
  font-size: 12px;
  color: var(--gv-text-dim);
}
.field-action {
  display: flex;
  align-items: flex-end;
}
</style>

<template>
  <div class="page-stack" data-testid="view-audit">
    <div class="surface">
      <div class="surface-header">{{ t('nav.audit') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
          <el-table :data="items" data-testid="audit-table" stripe max-height="640">
            <el-table-column prop="id" :label="t('audit.id')" width="80" />
            <el-table-column :label="t('audit.when')" width="168">
              <template #default="{ row }">{{ fmtWhen(row.ms) }}</template>
            </el-table-column>
            <el-table-column prop="actor" :label="t('audit.actor')" width="120" show-overflow-tooltip />
            <el-table-column :label="t('audit.action')" min-width="180">
              <template #default="{ row }">{{ humanAction(row.action) }}</template>
            </el-table-column>
            <el-table-column prop="target" :label="t('audit.target')" width="140" show-overflow-tooltip />
            <el-table-column :label="t('audit.detail')" min-width="220">
              <template #default="{ row }">
                <span class="mono muted detail">{{ fmtDetail(row.detail) }}</span>
              </template>
            </el-table-column>
          </el-table>
        </div>
      </div>
    </div>
  </div>
</template>
<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
const { t } = useI18n()
const items = ref([])
const tick = inject('refreshTick', ref(0))

function humanAction(a) {
  return String(a || '—').replace(/_/g, ' ')
}
function fmtWhen(ms) {
  const n = Number(ms)
  if (!Number.isFinite(n) || n <= 0) return '—'
  try {
    return new Date(n).toLocaleString(undefined, {
      month: 'numeric',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    })
  } catch {
    return String(ms)
  }
}
function fmtDetail(d) {
  if (!d) return '—'
  if (typeof d === 'string') return d
  try {
    return JSON.stringify(d)
  } catch {
    return String(d)
  }
}
async function load() {
  const j = await api('audit')
  items.value = j.items || []
}
onMounted(load)
watch(tick, load)
</script>
<style scoped>
.detail {
  display: inline-block;
  max-width: 100%;
  white-space: normal;
  word-break: break-word;
  line-height: 1.4;
}
</style>

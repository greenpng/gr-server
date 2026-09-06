<template>
  <div class="page-stack" data-testid="view-cluster">
    <div class="surface">
      <div class="surface-header">{{ t('nav.cluster') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
          <el-table :data="nodes" data-testid="cluster-table" stripe empty-text="—">
            <el-table-column prop="node_id" :label="t('cluster.node')" min-width="140" show-overflow-tooltip />
            <el-table-column prop="advertise" :label="t('cluster.advertise')" min-width="140" show-overflow-tooltip />
            <el-table-column prop="product_version" :label="t('cluster.version')" width="120" />
            <el-table-column :label="t('cluster.load')" min-width="240">
              <template #default="{ row }">
                <div class="load-bits">
                  <span v-for="b in loadBits(row)" :key="b.k" class="load-chip">
                    {{ b.k }} {{ b.v }}
                  </span>
                  <span v-if="!loadBits(row).length" class="muted">—</span>
                </div>
              </template>
            </el-table-column>
          </el-table>
        </div>
        <div v-if="!nodes.length" class="empty-state">{{ t('cluster.empty') }}</div>
      </div>
    </div>
  </div>
</template>
<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
const { t } = useI18n()
const nodes = ref([])
const tick = inject('refreshTick', ref(0))

function loadBits(row) {
  const l = row.load || {}
  const bits = []
  if (l.analyze_workers != null) bits.push({ k: t('cluster.analyze'), v: l.analyze_workers })
  if (l.cpu_pct != null) bits.push({ k: t('cluster.cpu'), v: `${Number(l.cpu_pct).toFixed(1)}%` })
  if (l.mem_pct != null) bits.push({ k: t('cluster.mem'), v: `${Number(l.mem_pct).toFixed(1)}%` })
  if (l.inflight != null) bits.push({ k: t('cluster.inflight'), v: l.inflight })
  return bits
}

async function load() {
  const j = await api('cluster')
  nodes.value = j.nodes || []
}
onMounted(load)
watch(tick, load)
</script>
<style scoped>
.load-bits {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.load-chip {
  font-size: 12px;
  color: var(--gv-text-muted);
  background: rgba(148, 163, 184, 0.08);
  border: 1px solid var(--gv-border);
  border-radius: 999px;
  padding: 2px 8px;
}
</style>

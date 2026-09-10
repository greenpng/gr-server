<template>
  <div class="app-shell" :class="{ 'drawer-open': drawerOpen }" data-testid="app-shell">
    <div class="sidebar-backdrop" @click="drawerOpen = false" />
    <aside class="sidebar">
      <div class="sidebar-brand">
        <div class="brand-mark">gr</div>
        <div class="brand-text">
          <strong>{{ t('app.brand') }}</strong>
          <div class="ver mono">{{ me?.version || '—' }}</div>
        </div>
      </div>

      <div class="nav-scroll" data-testid="sidebar">
        <div class="nav-section">{{ t('nav.section_product') }}</div>
        <div
          v-for="item in productNav"
          :key="item.path"
          class="nav-item"
          :class="{ 'is-active': isActive(item.path) }"
          :data-testid="item.testId"
          @click="go(item.path)"
        >
          <el-icon><component :is="item.icon" /></el-icon>
          <span>{{ t(item.labelKey) }}</span>
        </div>

        <div class="nav-section">{{ t('nav.section_platform') }}</div>
        <div
          v-for="item in platformNav"
          :key="item.path"
          class="nav-item"
          :class="{ 'is-active': isActive(item.path) }"
          :data-testid="item.testId"
          @click="go(item.path)"
        >
          <el-icon><component :is="item.icon" /></el-icon>
          <span>{{ t(item.labelKey) }}</span>
        </div>
      </div>

      <div class="sidebar-foot">
        <div class="user-chip">
          <span class="mono" data-testid="whoami">{{ me?.username || '—' }}</span>
          <el-button size="small" text data-testid="logout-btn" @click="logout">{{ t('app.logout') }}</el-button>
        </div>
      </div>
    </aside>

    <div class="main-col">
      <header class="topbar">
        <div style="display:flex;align-items:center;gap:10px;min-width:0">
          <el-button class="menu-toggle" text data-testid="menu-toggle" @click="drawerOpen = !drawerOpen">
            <el-icon :size="20"><Menu /></el-icon>
          </el-button>
          <div class="topbar-title">
            <h1>{{ title }}</h1>
            <p>{{ subtitle }}</p>
          </div>
        </div>
        <div class="topbar-actions">
          <el-select
            v-model="lang"
            size="small"
            style="width:118px"
            data-testid="lang-select"
            @change="onLang"
          >
            <el-option label="English" value="en" />
            <el-option label="中文" value="zh" />
          </el-select>
          <el-tag effect="dark" round :type="probeOk ? 'success' : 'danger'" data-testid="probe-pill">
            <span class="status-dot" :class="probeOk ? 'ok' : 'bad'" />
            {{ probeOk ? t('app.probe_ok') : t('app.probe_bad') }}
          </el-tag>
          <el-button size="small" data-testid="refresh-btn" @click="refresh">{{ t('app.refresh') }}</el-button>
        </div>
      </header>
      <main class="page">
        <router-view :key="tick" />
      </main>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, provide, markRaw } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  Odometer,
  Link,
  DataAnalysis,
  SetUp,
  Connection,
  Timer,
  Cpu,
  Box,
  Setting,
  Share,
  DataLine,
  Document,
  InfoFilled,
  Menu,
  Histogram,
  Key,
} from '@element-plus/icons-vue'
import { api, setAuthedFlag } from '../api'
import { setStoredLocale } from '../i18n'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const me = ref(null)
const probeOk = ref(false)
const tick = ref(0)
const lang = ref(locale.value)
const drawerOpen = ref(false)
provide('refreshTick', tick)

const productNav = [
  { path: '/dashboard', labelKey: 'nav.overview', icon: markRaw(Odometer), testId: 'nav-dashboard' },
  { path: '/sites', labelKey: 'nav.sites', icon: markRaw(Link), testId: 'nav-sites' },
  { path: '/results', labelKey: 'nav.results', icon: markRaw(DataAnalysis), testId: 'nav-results' },
  { path: '/strategies', labelKey: 'nav.strategies', icon: markRaw(SetUp), testId: 'nav-strategies' },
  { path: '/integrations', labelKey: 'nav.integrations', icon: markRaw(Connection), testId: 'nav-integrations' },
  { path: '/sdk', labelKey: 'nav.sdk', icon: markRaw(Key), testId: 'nav-sdk' },
  { path: '/retention', labelKey: 'nav.retention', icon: markRaw(Timer), testId: 'nav-retention' },
]
const platformNav = [
  { path: '/workers', labelKey: 'nav.workers', icon: markRaw(Cpu), testId: 'nav-workers' },
  { path: '/config', labelKey: 'nav.config', icon: markRaw(Setting), testId: 'nav-config' },
  { path: '/modules', labelKey: 'nav.modules', icon: markRaw(Box), testId: 'nav-modules' },
  { path: '/cluster', labelKey: 'nav.cluster', icon: markRaw(Share), testId: 'nav-cluster' },
  { path: '/loadbalance', labelKey: 'nav.loadbalance', icon: markRaw(Histogram), testId: 'nav-loadbalance' },
  { path: '/metrics', labelKey: 'nav.metrics', icon: markRaw(DataLine), testId: 'nav-metrics' },
  { path: '/audit', labelKey: 'nav.audit', icon: markRaw(Document), testId: 'nav-audit' },
  { path: '/status', labelKey: 'nav.status', icon: markRaw(InfoFilled), testId: 'nav-status' },
]

const title = computed(() => {
  const key = route.path.replace(/^\//, '') || 'dashboard'
  return t(`meta.${key}.title`)
})
const subtitle = computed(() => {
  const key = route.path.replace(/^\//, '') || 'dashboard'
  return t(`meta.${key}.subtitle`)
})

function isActive(path) {
  return route.path === path
}
function go(path) {
  drawerOpen.value = false
  if (route.path !== path) router.push(path)
}

function onLang(v) {
  locale.value = v
  setStoredLocale(v)
  window.location.reload()
}

async function load() {
  try {
    me.value = await api('me')
  } catch {
    me.value = null
  }
  try {
    const j = await api('probe/health')
    probeOk.value = !!j.ok
  } catch {
    probeOk.value = false
  }
}
function refresh() {
  tick.value++
  load()
}
async function logout() {
  try {
    await api('logout', { method: 'POST', body: '{}' })
  } catch {
    /* ignore */
  }
  setAuthedFlag(false)
  router.push('/login')
}
onMounted(load)
</script>

<style scoped>
.menu-toggle {
  display: none;
  color: var(--gv-text) !important;
}
@media (max-width: 960px) {
  .menu-toggle {
    display: inline-flex;
  }
}
</style>

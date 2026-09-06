import { createRouter, createWebHashHistory } from 'vue-router'
import { isAuthedFlag, setAuthedFlag, api } from './api'
import Login from './views/Login.vue'
import Layout from './views/Layout.vue'
import Dashboard from './views/Dashboard.vue'
import Sites from './views/Sites.vue'
import Results from './views/Results.vue'
import Strategies from './views/Strategies.vue'
import Integrations from './views/Integrations.vue'
import Retention from './views/Retention.vue'
import Workers from './views/Workers.vue'
import Modules from './views/Modules.vue'
import Cluster from './views/Cluster.vue'
import LoadBalance from './views/LoadBalance.vue'
import Metrics from './views/Metrics.vue'
import Audit from './views/Audit.vue'
import Status from './views/Status.vue'
import Sdk from './views/Sdk.vue'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/login', name: 'login', component: Login, meta: { public: true } },
    {
      path: '/',
      component: Layout,
      children: [
        { path: '', redirect: '/dashboard' },
        { path: 'dashboard', component: Dashboard },
        { path: 'sites', component: Sites },
        { path: 'results', component: Results },
        { path: 'strategies', component: Strategies },
        { path: 'integrations', component: Integrations },
        { path: 'sdk', component: Sdk },
        { path: 'retention', component: Retention },
        { path: 'workers', component: Workers },
        { path: 'modules', component: Modules },
        { path: 'cluster', component: Cluster },
        { path: 'loadbalance', component: LoadBalance },
        { path: 'metrics', component: Metrics },
        { path: 'audit', component: Audit },
        { path: 'status', component: Status },
      ],
    },
  ],
})

router.beforeEach(async (to) => {
  if (to.meta.public) {
    if (to.path === '/login' && isAuthedFlag()) {
      try {
        await api('me')
        return { path: '/dashboard' }
      } catch {
        setAuthedFlag(false)
      }
    }
    return true
  }
  if (!isAuthedFlag()) {
    try {
      await api('me')
      setAuthedFlag(true)
      return true
    } catch {
      return { path: '/login', query: { redirect: to.fullPath } }
    }
  }
  return true
})

export default router

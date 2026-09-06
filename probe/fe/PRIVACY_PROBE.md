# V5 静默探测 · 权限红线

## 原则

V5 主程序是**探测 + 静默收集**。默认安装**不得**触发浏览器向用户申请授权。

| 禁止（探测侧主动） | 允许 |
|--------------------|------|
| `getUserMedia` / `getDisplayMedia` | `enumerateDevices` **仅计数**（无 label） |
| `geolocation.getCurrentPosition/watch` | 只读 `Notification.permission` |
| `clipboard.read*` | WebRTC **无媒体轨**（host hash） |
| `Notification.requestPermission` | OfflineAudio / WebGL / 时序 |
| 为指纹打开麦/相机/定位 | **被动观察**站点业务已打开的麦/相机 |

## 监控

`gr.privacy_guard.js` 安装后：

- **拦截**探测栈对禁止 API 的调用 → `blocked_n` + ops `privacy_guard`
- **观察**业务页调用 getUserMedia 等 → `site_media_access_observed`

字段经 `B28_permissions_media` 上报：`privacy_guard_*`、`site_*`。

## 批次

`B28_permissions_media`：`gr_permissions_readonly_v2`  
不再 `permissions.query(camera|mic|geo|clipboard)`。

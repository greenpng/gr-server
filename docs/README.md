# GR Server 用户文档

| 文档 | 内容 |
|------|------|
| [01-install.md](01-install.md) | 安装：二进制树 / `--with-docker` 数据层 / 环境变量分组 |
| [02-deploy-probe-sdk.md](02-deploy-probe-sdk.md) | 网站嵌入探测脚本（三种加载方式）+ 六语言后端 SDK |
| [03-modules.md](03-modules.md) | 模块（.so）清单、签名、热更与回滚 |
| [04-operations.md](04-operations.md) | 备份、升级、回滚、签名与加固参考 |
| [05-local-dev.md](05-local-dev.md) | 从源码构建、跑测试（开发者） |
| [reference/](reference/) | 部署与更新规则全文、加固细节 |

安装/更新的唯一入口是本仓：`install/install.sh` 与 `install/release/*.sh`
拉本仓 GitHub Release 资产。Docker 是可选运行时，不是更新通道。

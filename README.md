# Token Monitor Duo

使用 Rust + Tauri 桌面客户端，在玻璃窗口中查看同一个 Pro 额度周期内，每位成员、每台设备的 Codex Token 和 API 估算费用。支持 macOS 和 Windows 原生客户端，服务端继续使用兼容原版协议的 Node Hub。

[下载安装包](https://github.com/mrxiuer/token-monitor/releases) · [使用与统计口径](docs/duo.md) · [反馈问题](https://github.com/mrxiuer/token-monitor/issues)

## 主要功能

- 连接 Hub 时填写用户名，同名设备自动归到同一成员；成员与设备列表来自 Hub，可随实际使用人数变化。
- 「本轮」按共享 Pro 的额度窗口统计，并保留历史周期。当天、当月、全部仍可切换查看。
- 自动观察周额度和重置卡变化。重置时刻无法确认时，把边界区间用量单独列为「待确认」，获得可靠边界后重新计算。
- 点击成员、设备、模型逐层展开，查看分类 Token、估算费用、客户端版本与同步时间。
- 保留原版的玻璃材质、配色与等宽字体，主界面常驻「×」。收起窗口后继续采集和同步，通过托盘或快捷键恢复。
- 数值账本在本机保存，断网后继续采集并重试同步；未知模型价格、未上传或不完整的数据会明确标示。

## 使用

各台设备安装本 fork 的客户端，填写相同的 Hub URL 和共享密钥。属于同一个人的电脑填写相同用户名，每台电脑保留各自唯一的设备 ID。已有原版设置时，首次运行会尝试读取 Hub、设备 ID 与外观选项；原版数据目录保持独立。

Hub 从有效的 Codex Pro 主周额度中识别周期，并固定共享账号标识。同一账号可由多台设备提供额度读数。设置中的额度信息为只读，无需手动指定周期。周期不等于自然周，也不总是完整七天；重置卡的具体处理见[统计口径](docs/duo.md#额度周期与重置卡)。

**本版统计设备上的全部可读取 Codex 日志，不跟踪账号切换，也不分配个人预算。** 费用是模型 API 价格估算，不能直接换算为订阅扣费或官方额度百分比。共享密钥允许读取全部成员的数据，用户名不提供独立权限。

## 构建与部署

开发需要 Node.js 22.15+、Rust stable，以及对应平台的 [Tauri 构建依赖](https://v2.tauri.app/start/prerequisites/)。安装包运行时不需要 Node.js、Rust 或 Electron。macOS 使用系统 WebKit，Windows 使用 WebView2。

```sh
npm ci
npm run verify
npm run desktop:test
npm start
```

macOS Apple Silicon 使用 `npm run desktop:build`，生成 `desktop/src-tauri/target/release/bundle/macos/Token Monitor Duo.app`。Windows x64 使用 `npm run desktop:build:win`，生成 NSIS 安装包。GitHub Actions 的 `Duo Build` 在这两个平台执行原生测试与打包。构建依赖与命令详见[开发说明](docs/duo.md#开发与发布)。

安装包采用本 fork 的手动更新入口。Mac 使用 ad-hoc 签名，未经过 Apple 公证；Windows 使用未签名安装包。独立应用标识避免覆盖原版，日常使用时应先退出旧客户端，避免同一设备重复上报。

服务端运行 `npm run hub`。升级前备份数据与服务配置，保留既有数据路径。升级 Node Hub 后，旧版客户端仍可上传日／月／累计数据；准确的周期和新增模型明细需要各台设备安装新版客户端。

## 原项目与许可

基于 [Javis603/token-monitor](https://github.com/Javis603/token-monitor)，遵循原 [MIT 许可证](LICENSE)。完整的上游功能与开发说明保留在 [原版 README](README.upstream.md)，其中下载、赞助、社区入口属于上游项目。原 Electron、Tokscale 与其他提供方源码保留供兼容和合并上游使用；本版原生安装包使用 Rust 采集器与数值价格目录。

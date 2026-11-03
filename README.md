# Token Monitor · 双人 Codex 统计版

在原版 Token Monitor 的玻璃窗口中，查看同一个 Pro 额度周期内，每位成员、每台设备产生的 Codex Token 与 API 估算费用。

[下载安装包](https://github.com/mrxiuer/token-monitor/releases) · [使用与统计口径](docs/duo.md) · [反馈问题](https://github.com/mrxiuer/token-monitor/issues)

## 主要功能

- 连接同一个 Node Hub 时填写用户名，同名设备自动归组。成员可以改名，设备可以重新分配，历史仍按稳定的成员 ID 汇总。
- 「额度周」使用统一的 Pro 周额度起止时间，支持本轮、历史周期与 CSV 导出。保留当天、当月、全部统计。
- 按本地用量事件的时间拆分跨周期会话，分别计算输入、输出与缓存费用。无法读取的来源、未上传的旧版设备和未计价 Token 会单独标明。
- 主界面始终显示「×」。点击收起窗口，后台继续采集与同步；完全退出仍使用托盘菜单。
- 保留原版玻璃材质、透明度、字体、配色和窗口设置。常用界面围绕成员、设备、Codex 和 Hub；其余高级功能默认关闭或收起。

## 使用

三台设备都安装本 fork 的定制客户端，使用相同的 Hub URL 和密钥。属于同一个人的电脑填写相同用户名，设备 ID 保持各自唯一。不要把 Hub 密钥写进仓库或公开截图。

Hub 首次读取到有效的 Codex Pro 周额度后，会固定其设备及账号标识作为周期来源，避免别的 Plus 账号改变统计边界。也可以在设置中选择来源或手动校正起止时间。没有可靠的新额度信息时，会保留结束的周期并提示等待，不推算重置时间。

本版按设备采集所有 Codex 使用记录，**不区分设备上切换过的账号，也不分配个人预算或订阅额度比例**。显示的费用是模型 API 价格估算，不是订阅实际扣费，也不能直接换算为官方额度百分比。

## 构建与部署

需要 Node.js 22.15+。`npm ci` 后运行 `npm run verify`，开发窗口使用 `npm start`。Mac Apple Silicon 使用 `npm run dist:duo:mac`，Windows x64 使用 `npm run dist:duo:win`；GitHub Actions 的 `Duo Build` 也会生成这两种安装包。

定制构建使用独立版本号和本 fork 的发布入口，采用手动下载安装方式。Mac 包使用本地临时签名，未经过 Apple 公证；Windows 包没有原作者的 SignPath 签名。系统首次打开时可能要求确认来源。不要使用原版自动更新覆盖定制版。

服务端继续使用 Node Hub（`npm run hub`），升级前备份数据和配置，保留原数据文件位置。客户端和 Hub 两端都需升级才能使用成员及周期配置。旧版设备可继续上传原来的日/月/总量，但不会提供周期统计。

## 原项目与许可

基于 [Javis603/token-monitor](https://github.com/Javis603/token-monitor)，遵循原 [MIT 许可证](LICENSE)。完整的原版功能、提供方说明和开发文档保留在 [原版 README](README.upstream.md)；其中原作者下载、赞助、社区入口属于上游项目。Tokscale 仍使用原有固定来源和校验，不改写其归属。

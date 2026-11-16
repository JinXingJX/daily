# Journal · 日记

一个独立的 Daily 日记插件。直接写字、粘贴文字，先记录，再让 Agent 帮你整理。

## 安装

在仓库根目录执行：

```sh
npm install
npm --prefix examples/daily-journal-plugin run build
```

在 Daily 的 **设置 → 插件 → 安装本地插件** 中，填入本目录的绝对路径：

```text
/Users/jinxing/Desktop/learn/rust/daliy/daliy/examples/daily-journal-plugin
```

开发模式：`npm --prefix examples/daily-journal-plugin run dev`，安装时填写开发地址 `http://127.0.0.1:5175`。已经安装后重新构建，可使用 Daily 的「重新加载插件」。构建后的 `.daily-plugin/dist/` 必须与清单一起分发。

## 使用

- **自由写作**：原生纯文本编辑、中文输入法、长文粘贴、标题、日期、搜索、专注模式。450ms 防抖自动保存，`⌘/Ctrl + S` 立即保存。保存失败时保留内存中的原文，并保持 Daily 的未保存关闭提醒。
- **打字反馈**：安静、墨迹涟漪、微光。反馈跟随实际输入位置，输入法组词时不触发；尊重系统减少动态效果设置。
- **写作足迹**：最近 13 周和全年热力图，点击日期筛选或补写。统计当天所有日记当前保留的正文字数，删字会减少统计；中文每字计一，英文/数字每词计一，标点和 emoji 不计。不是累计按键数。
- **摘要**：提炼每篇日记的摘要和要点，保留原文。
- **社交图文**：生成可编辑的标题、正文和配图提示词，复制文案或导出完整 PNG 观点卡片（长文自动分页）。不自动发布到社交平台。
- **素材缺口**：提示需要查找的资料、补充的个人细节或配图。支持搜索素材、添加摘录、读取公开链接并总结；网页受限时可以手动粘贴正文。来源摘要保留对应链接。
- **AI 配图**：点击「生成配图」时才调用 Daily 已配置的 OpenAI 图片服务，按服务商计费。生成结果保存到本地，可预览、下载。没有图片密钥时会显示设置提示；观点卡片导出不需要图片服务。
- **待办**：分别提取计划、待验证事项、预测复盘，展示原文依据及原文明确支持的日期。建议经点击加入后进入插件待办列表，可以勾选完成、返回来源日记；不会创建定时提醒或日历事件。

Agent 使用 Daily 设置中的默认模型，不另外配置文本 API。只有点击整理、读取链接或生成图片时才发送对应材料。后台任务由插件的 `activate()` 注册，切换页面不影响执行；任务失败可明确重试，中断可继续。图片等外部调用使用宿主操作日志，不会自动重复扣费；宿主判定结果不确定时需要重新发起新任务。

日记修改后会提示已有整理结果可能过期。补充材料后，重新运行对应整理动作即可更新结果。

## 数据

数据按工作区分别保存在：

```text
.daily/plugin-files/local.daily_journal/entries/<entry-id>.json
.daily/plugin-files/local.daily_journal/images/<task-id>/...
```

偏好保存在插件 storage，任务保存在宿主的持久任务数据库。JSON 文件包含原文、材料、整理结果和待办。可用工具栏导出 Markdown；损坏的 JSON 会保留原文件并提示，不会覆盖。切换到其他工作区会打开另一本日记。日记没有加密；AI 处理会将选择的日记和附加材料发给用户配置的模型服务。

## 验证

```sh
npm --prefix examples/daily-journal-plugin test
npm --prefix examples/daily-journal-plugin run build
npm run typecheck
npm run build
```

插件只使用 `@daily/plugin-sdk`，视图挂载在 Shadow DOM；不需要修改 Daily 主程序、生成协议或增加宿主 RPC。

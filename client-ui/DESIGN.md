# client-ui 设计规范

适用于 iPhone、iPad、visionOS (嵌入 `NshAppView`) 与桌面浏览器 (Daemon 托管)。依据 NSH Design Guides, 只记录这个应用自己的决定。

## 目标平台

- **触屏 (iOS、iPadOS、visionOS、手机浏览器):** 控件不小于 44pt, 正文 16pt 起, 底部标签栏, 查看器是全屏层。
- **桌面与 Web (`platform.desktop`):** 紧凑控件 (32pt), 顶部标签栏, 键盘优先, hover 与可见焦点环。
- 判断只用 `platform.desktop` / `platform.os`, 不用屏幕宽度猜设备。

## Token

颜色只通过 `theme.json` 的 token 使用, 代码里没有十六进制或调色板类。

| Token | 用途 |
|---|---|
| `primary` / `primary-foreground` | 主操作、选中的标签、进度 |
| `success` | 已完成、Engine 运行中 |
| `warning` | Engine 启动中、提示条 |
| `destructive` / `destructive-foreground` | 失败、危险操作、错误文字 |
| `muted` / `muted-foreground` | 次要文字、占位、轨道 |
| `card`、`border`、`input`、`ring` | 表面、分隔线、输入框边框、焦点环 |

浅色与深色各一组值, 跟随系统。圆角统一 `rounded-(--radius)` (12pt)。

## ui 层

`src/ui/` 是应用自己的组件层, 页面只组合它们, 不直接用 `nsh/base` 的控件, 也不写颜色、圆角、字号:

`Icon` (自绘线条图标)、`Button` / `IconButton` (primary、secondary、ghost、destructive)、`TextField` / `NumberField` / `SwitchField` / `SelectField`、`Card`、`Badge`、`Banner`、`Empty`、`Spinner` / `Meter`、`Segmented` / `NavTabs`、`ToastHost`。

页面需要而 `ui` 没有的外观, 先加成 `ui` 的变体再用。

## 状态

每个会变的东西都有可见的状态, 而且不只靠颜色: 任务状态是文字 (排队中 · 翻译中 · 已完成 · 失败 · 已取消) 加色标; 表单错误是红字说明; 禁用是 `disabled:opacity-40` 加不可按; 键盘高亮行有 `bg-accent`, 焦点用 `focus-visible:ring-2`。

## 键盘

| 位置 | 按键 | 行为 |
|---|---|---|
| 任务列表 | ↑ ↓ | 移动高亮 (到头停住, 移出屏幕才滚动) |
| 任务列表 | Enter / 空格 | 打开高亮的已完成任务 |
| 任务列表 | Delete | 移除高亮项 |
| 查看器 | ← ↑ / → ↓ | 上一张 / 下一张已完成任务 |
| 查看器 | 空格 | 译文与原文互换 (与 PDF 阅读器的空格一致) |
| 查看器 | Esc | 关闭 |
| 表单 | Enter | 提交 (配对页) |

输入框获得焦点时, 方向键和空格交给输入框, 不触发上面的绑定 (NSH keymap 的默认行为)。

## 动效清单

| 变化 | 工具 | 说明 |
|---|---|---|
| 按下反馈 | `active:opacity-70` | 所有按钮、列表行 |
| 开关滑块 | `transition-transform` / `transition-colors` | `SwitchField` 的轨道与滑块 |
| 分段控件与标签的指示条 | `Tabs` 的 `indicatorClass`, 原生滑动 | `Segmented` |
| 列表增删与状态变化 | 无 | 行原地更新; 批量任务每秒都在变, 动画只会增加噪音 |
| 提示条 (toast) 出现与消失 | 无 | 3.5 秒自动收起, 点按立即收起 |
| 查看器进入 | 无 | 全屏层直接出现, 切图不做转场, 方向键连按要立即响应 |

尊重「减少动态效果」: 以上动效都由 NSH 的类与原生指示条提供, 系统设置开启时由 NSH 处理。

## 完成检查

每次改界面按 Design Guides 的 Finishing checklist 看浅色与深色截图: 圆角、字号、间距、对齐、颜色、hover、按下、各种状态、动效。截图存 `/tmp/nsh-ui-shots` 对照。

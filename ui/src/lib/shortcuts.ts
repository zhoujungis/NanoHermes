/**
 * 快捷键清单：提示面板（Ctrl+/）与命令面板共用的唯一数据源。
 *
 * 只收录真正实现了的按键，改按键处理时同步改这里，避免面板给出按不动的假快捷键。
 */

export interface ShortcutItem {
  /** 组合键，按显示顺序拆开（如 ["Ctrl", "K"]、["Shift", "Enter"]）。 */
  keys: string[];
  desc: string;
}

export interface ShortcutGroup {
  group: string;
  items: ShortcutItem[];
}

export const SHORTCUTS: ShortcutGroup[] = [
  {
    group: "全局",
    items: [
      { keys: ["Ctrl", "K"], desc: "打开命令面板" },
      { keys: ["Ctrl", "N"], desc: "新建任务" },
      { keys: ["Ctrl", "F"], desc: "在会话内查找" },
      { keys: ["Ctrl", "B"], desc: "显示 / 隐藏左侧任务栏" },
      { keys: ["Ctrl", "J"], desc: "显示 / 隐藏右侧结果区" },
      { keys: ["Ctrl", ","], desc: "打开设置" },
      { keys: ["Ctrl", "/"], desc: "打开 / 关闭本面板" },
      { keys: ["Esc"], desc: "逐层关闭浮层（命令面板 → 设置 → 菜单 → 查找 → 预览）" },
    ],
  },
  {
    group: "输入区",
    items: [
      { keys: ["Enter"], desc: "发送消息" },
      { keys: ["Shift", "Enter"], desc: "换行，不发送" },
      { keys: ["↑", "↓"], desc: "在首行 / 末行翻阅输入历史" },
      { keys: ["@"], desc: "唤起文件选择器，把选中路径插入输入框" },
    ],
  },
  {
    group: "会话内查找",
    items: [
      { keys: ["Enter"], desc: "跳到下一个命中" },
      { keys: ["Shift", "Enter"], desc: "跳到上一个命中" },
      { keys: ["Esc"], desc: "关闭查找条" },
    ],
  },
  {
    group: "命令面板",
    items: [
      { keys: ["↑", "↓"], desc: "上下选择命令" },
      { keys: ["Enter"], desc: "执行选中的命令" },
      { keys: ["Esc"], desc: "关闭命令面板" },
    ],
  },
  {
    group: "设置页",
    items: [
      { keys: ["Tab"], desc: "在字段与选项组之间依次移动焦点" },
      { keys: ["←", "→"], desc: "在主题 / 模式 / 执行位置等选项组内切换" },
      { keys: ["Esc"], desc: "保存并关闭设置" },
    ],
  },
];

import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  activeTask,
  closePalette,
  createTask,
  exportCurrent,
  insertSnippet,
  openFolderPick,
  resetCurrent,
  resolveModel,
  resolvePersona,
  setMode,
  setPanelTab,
  setTaskLocation,
  setTaskModel,
  setTaskPersona,
  stop,
  taskIsCloud,
  togglePin,
  useAppState,
} from "../lib/store";
import { Icon, type IconName } from "./Icon";
import { setTheme, useTheme, THEME_META } from "../lib/theme";

interface Cmd {
  id: string;
  group: string;
  label: string;
  hint?: string;
  icon: IconName;
  /** 条件不成立时不出现在列表里。 */
  hide?: boolean;
  run: () => void;
}

/** 模式入口：值与后端 PermissionMode 对应，标签与输入区保持一致。 */
const MODES: Array<[string, string]> = [
  ["default", "Agent"],
  ["accept-edits", "Accept"],
  ["plan", "Plan"],
  ["goal", "Goal"],
  ["debug", "Debug"],
];

/**
 * Ctrl+K 命令面板：把界面上所有需要点几下的动作收进一个键盘可搜索的列表。
 * 每一项都直接调用 store 里已有的动作，不新增任何后端能力。
 */
export function CommandPalette({
  onOpenSettings,
  onOpenShortcuts,
  onToggleSidebar,
  onToggleAside,
}: {
  onOpenSettings: () => void;
  onOpenShortcuts: () => void;
  onToggleSidebar: () => void;
  onToggleAside: () => void;
}) {
  const st = useAppState();
  const task = activeTask();
  const cloud = taskIsCloud(task);
  const hasChat = !!task?.msgs.length;
  const theme = useTheme();

  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  const all = useMemo<Cmd[]>(() => {
    const id = task?.id;
    const items: Cmd[] = [
      {
        id: "new-local",
        group: "任务",
        icon: "laptop",
        label: "新建本机任务",
        hint: "在当前进程内执行",
        run: () => void createTask(false, "local"),
      },
      {
        id: "new-cloud",
        group: "任务",
        icon: "cloud",
        label: "新建云端任务",
        hint: "转发到远程 daemon",
        run: () => void createTask(false, "cloud"),
      },
      {
        id: "stop",
        group: "任务",
        icon: "square",
        label: "停止正在执行的任务",
        hint: "当前步骤结束后生效",
        // 本机与云端都支持：云端在读到下一个 NDJSON 事件时发现停止标志并断开。
        hide: !st.busy,
        run: () => void stop(),
      },
      {
        id: "pin",
        group: "任务",
        icon: "pin",
        label: task?.pinned ? "取消置顶当前任务" : "置顶当前任务",
        hint: "置顶的任务排在列表最前",
        hide: !id,
        run: () => id && togglePin(id),
      },
      {
        id: "loc-cloud",
        group: "执行位置",
        icon: "cloud",
        label: "把当前任务切到云端执行",
        hide: !id || cloud,
        run: () => id && void setTaskLocation(id, "cloud"),
      },
      {
        id: "loc-local",
        group: "执行位置",
        icon: "laptop",
        label: "把当前任务切回本机执行",
        hide: !id || !cloud,
        run: () => id && void setTaskLocation(id, "local"),
      },
      {
        id: "ws",
        group: "执行位置",
        icon: "folder",
        label: "更改本机任务的工作区文件夹",
        hide: !id || cloud,
        run: () =>
          openFolderPick({ forTask: id ?? null, initial: task?.workspace || "" }),
      },
      {
        id: "tab-tree",
        group: "结果区",
        icon: "file",
        label: "结果区：工作区文件",
        hide: cloud,
        run: () => setPanelTab("tree"),
      },
      {
        id: "tab-artifacts",
        group: "结果区",
        icon: "box",
        label: "结果区：本任务产物",
        run: () => setPanelTab("artifacts"),
      },
      {
        id: "export",
        group: "结果区",
        icon: "download",
        label: "导出对话为 Markdown",
        hide: !hasChat,
        run: () => void exportCurrent(),
      },
      {
        id: "reset",
        group: "结果区",
        icon: "refresh",
        label: "清空当前任务的对话上下文",
        hide: !hasChat,
        run: () => void resetCurrent(),
      },
      {
        id: "settings",
        group: "界面",
        icon: "gear",
        label: "打开设置",
        run: onOpenSettings,
      },
      {
        id: "shortcuts",
        group: "界面",
        icon: "keyboard",
        label: "键盘快捷键",
        hint: "Ctrl+/",
        run: onOpenShortcuts,
      },
      {
        id: "sidebar",
        group: "界面",
        icon: "panelLeft",
        label: "显示 / 隐藏左侧任务栏",
        run: onToggleSidebar,
      },
      {
        id: "aside",
        group: "界面",
        icon: "panelRight",
        label: "显示 / 隐藏右侧结果区",
        run: onToggleAside,
      },
    ];
    if (!cloud) {
      for (const [value, label] of MODES) {
        items.push({
          id: "mode-" + value,
          group: "模式",
          icon: "shield",
          label: "切换到 " + label + " 模式",
          hide: st.mode === value,
          run: () => void setMode(value),
        });
      }
      // 与输入区的模型下拉同源：这里也把当前任务的模型档案列出来。
      const currentModel = resolveModel(st.settings, task?.model)?.id || "";
      for (const m of st.settings?.models || []) {
        const name = m.name || m.model || m.id;
        items.push({
          id: "model-" + m.id,
          group: "模型",
          icon: "spark",
          label: "使用模型：" + name,
          hint: m.model && m.model !== name ? m.model : undefined,
          hide: !id || m.id === currentModel,
          run: () => id && void setTaskModel(id, m.id),
        });
      }
      // 与输入区的人设下拉同源：这里也把当前任务的人设列出来。
      const currentPersona = resolvePersona(st.settings, task?.persona)?.id || "";
      for (const p of st.settings?.personas || []) {
        const name = p.name || "未命名人设";
        items.push({
          id: "persona-" + p.id,
          group: "人设",
          icon: "user",
          label: "使用人设：" + name,
          hint: p.prompt.trim() ? undefined : "不附加任何指令",
          hide: !id || p.id === currentPersona,
          run: () => id && void setTaskPersona(id, p.id),
        });
      }
    }
    // 片段只把文本插进输入框，本机与云端任务都能用，所以不放在 !cloud 分支里。
    for (const s of st.settings?.snippets || []) {
      const name = s.name || "未命名片段";
      items.push({
        id: "snippet-" + s.id,
        group: "片段",
        icon: "bolt",
        label: "插入片段：" + name,
        hint: s.text.length > 40 ? s.text.slice(0, 40) + "…" : s.text || undefined,
        run: () => insertSnippet(s.text),
      });
    }
    for (const t of THEME_META) {
      items.push({
        id: "theme-" + t.id,
        group: "主题",
        icon: "theme",
        label: "主题：" + t.name,
        hint: t.hint,
        hide: theme === t.id,
        run: () => setTheme(t.id),
      });
    }
    return items.filter((c) => !c.hide);
  }, [st.busy, st.mode, st.settings, task, cloud, hasChat, theme, onOpenSettings, onOpenShortcuts, onToggleSidebar, onToggleAside]);

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return all;
    return all.filter((c) =>
      `${c.label} ${c.group} ${c.hint || ""}`.toLowerCase().includes(needle),
    );
  }, [all, q]);

  // 选中项永远落在有效范围内，搜索变化时回到第一项。
  const active = Math.min(sel, Math.max(0, filtered.length - 1));

  // 打开时接住焦点，关闭时还回去（面板是条件渲染，卸载即关闭）。
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => {
      const active = document.activeElement;
      if (!active || active === document.body || active.closest(".cmd-back")) {
        prev?.focus?.();
      }
    };
  }, []);

  useEffect(() => {
    list.current?.querySelector(".cmd-item.on")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const runAt = (n: number): void => {
    const c = filtered[n];
    if (!c) return;
    closePalette();
    c.run();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel(Math.min(active + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel(Math.max(active - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      runAt(active);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closePalette();
    }
  };

  return (
    <div
      className="cmd-back"
      role="dialog"
      aria-modal="true"
      aria-label="命令面板"
      onMouseDown={() => closePalette()}
    >
      <div className="cmd" onMouseDown={(e) => e.stopPropagation()}>
        <div className="cmd-in">
          <Icon name="search" />
          <input
            ref={input}
            value={q}
            spellCheck={false}
            placeholder="搜索命令…"
            aria-label="搜索命令"
            onChange={(e) => {
              setQ(e.target.value);
              setSel(0);
            }}
            onKeyDown={onKeyDown}
          />
          <span className="kbd">Esc</span>
        </div>
        <div className="cmd-list" ref={list}>
          {filtered.length === 0 && <div className="cmd-empty">没有匹配的命令</div>}
          {filtered.map((c, n) => (
            <Fragment key={c.id}>
              {(n === 0 || filtered[n - 1].group !== c.group) && (
                <div className="cmd-group">{c.group}</div>
              )}
              <button
                className={"cmd-item" + (n === active ? " on" : "")}
                onMouseMove={() => setSel(n)}
                onClick={() => runAt(n)}
              >
                <Icon name={c.icon} />
                <span className="cl">{c.label}</span>
                {c.hint && <span className="ch">{c.hint}</span>}
              </button>
            </Fragment>
          ))}
        </div>
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { Composer } from "./components/Composer";
import { FilePicker } from "./components/FilePicker";
import { FolderPicker } from "./components/FolderPicker";
import { ResultsPanel } from "./components/ResultsPanel";
import { SettingsPage } from "./components/SettingsPage";
import { ShortcutsPanel } from "./components/ShortcutsPanel";
import { Sidebar } from "./components/Sidebar";
import { TaskBar } from "./components/TaskBar";
import { TaskMenu } from "./components/TaskMenu";
import { TitleBar } from "./components/TitleBar";
import { Toast } from "./components/Toast";
import { Transcript } from "./components/Transcript";
import * as store from "./lib/store";
import { restoreWindowState, trackWindowState } from "./lib/windowState";

/** 面板显隐属于界面偏好，独立持久化，重启后保持用户上次的布局。 */
const LAYOUT_KEY = "nanohermes.layout.v1";
function readLayout(): { sidebar: boolean; aside: boolean } {
  try {
    const raw = JSON.parse(localStorage.getItem(LAYOUT_KEY) || "null");
    return { sidebar: raw?.sidebar !== false, aside: raw?.aside !== false };
  } catch {
    return { sidebar: true, aside: true };
  }
}

export default function App() {
  const st = store.useAppState();

  // 纯 UI 状态（面板显隐、浮层、就地编辑）留在组件里，业务状态在 store 里。
  const [showSidebar, setShowSidebar] = useState(() => readLayout().sidebar);
  const [showAside, setShowAside] = useState(() => readLayout().aside);
  const [showSettings, setShowSettings] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  // 同一个 editingId 可能来自侧栏双击或顶栏按钮；必须记住来源，否则两处会同时
  // 渲染输入框（双击侧栏标题却在顶栏弹出重命名）。
  const [editingWhere, setEditingWhere] = useState<"sidebar" | "bar" | null>(null);
  const [menu, setMenu] = useState<{ id: string; rect: DOMRect } | null>(null);
  /** 文件选择器：onPick 不传时选中即加入引用，传了则由调用方决定选中行为（@ 插入路径）。 */
  const [picker, setPicker] = useState<{
    rect: DOMRect;
    onPick?: (path: string) => void;
  } | null>(null);

  useEffect(() => {
    void store.boot();
  }, []);

  useEffect(() => store.wireEvents(), []);

  // 面板显隐变化时落盘；窗口尺寸/位置由主进程侧恢复并在拖动后记录。
  useEffect(() => {
    try {
      localStorage.setItem(
        LAYOUT_KEY,
        JSON.stringify({ sidebar: showSidebar, aside: showAside }),
      );
    } catch {
      /* 配额不足时忽略，不影响本次会话 */
    }
  }, [showSidebar, showAside]);

  useEffect(() => {
    void restoreWindowState();
    let stop: () => void = () => {};
    void trackWindowState().then((fn) => {
      stop = fn;
    });
    return () => stop();
  }, []);

  // 快捷键：Esc 按浮层层级逐层关闭，Ctrl+K 命令面板，Ctrl+N / , / B / J / 与顶栏按钮等价。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (e.key === "Escape") {
        if (st.paletteOpen) store.closePalette();
        else if (picker) setPicker(null);
        else if (showShortcuts) setShowShortcuts(false);
        else if (showSettings) setShowSettings(false);
        else if (menu) setMenu(null);
        else if (st.folderPick) store.closeFolderPick();
        else if (st.newTaskOpen) store.closeNewTask();
        else if (editingId) {
          setEditingId(null);
          setEditingWhere(null);
        } else if (st.findOpen) store.closeFind();
        else if (st.preview) store.closePreview();
        return;
      }
      if (!mod) return;
      const k = e.key.toLowerCase();
      // 设置页打开时它独占键盘；快捷键面板只保留 Ctrl+/ 用于收起，命令面板
      // 只保留 Ctrl+K。否则 Ctrl+F 会把焦点抢进被遮住的查找条。
      if (showSettings) return;
      if (showShortcuts && k !== "/") return;
      if (st.paletteOpen && k !== "k") return;
      if (k === "k") {
        e.preventDefault();
        if (st.paletteOpen) store.closePalette();
        else store.openPalette();
      } else if (k === "f") {
        // 会话内查找，替代 WebView 自带的页面查找。
        e.preventDefault();
        store.openFind();
      } else if (k === "n") {
        e.preventDefault();
        // 侧栏可见时先展示「本机 / 云端」选择；不可见时按默认位置直接新建。
        if (showSidebar) store.openNewTask();
        else void store.createTask();
      } else if (e.key === ",") {
        e.preventDefault();
        setShowSettings(true);
      } else if (k === "b") {
        e.preventDefault();
        setShowSidebar((v) => !v);
      } else if (k === "j") {
        e.preventDefault();
        setShowAside((v) => !v);
      } else if (e.key === "/") {
        e.preventDefault();
        // 快捷键提示面板：开着时再按一次收起。
        setShowShortcuts((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [picker, showSettings, showShortcuts, menu, editingId, editingWhere, st.preview, st.newTaskOpen, st.folderPick, st.paletteOpen, st.findOpen, showSidebar]);

  // 点击空白处收起浮层（锚点元素自身除外，否则刚点开就会被立刻关掉）。
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (!t) return;
      if (!t.closest(".picker") && !t.closest("[data-picker-anchor]")) setPicker(null);
      if (!t.closest(".menu") && !t.closest("[data-menu-anchor]")) setMenu(null);
      if (!t.closest(".newpick") && !t.closest(".newtask")) store.closeNewTask();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, []);

  return (
    <div className="shell">
      <TitleBar
        showSidebar={showSidebar}
        showAside={showAside}
        onToggleSidebar={() => setShowSidebar((v) => !v)}
        onToggleAside={() => setShowAside((v) => !v)}
        onOpenSettings={() => setShowSettings(true)}
      />

      <div className="work">
        <Sidebar
          hidden={!showSidebar}
          editingId={editingWhere === "sidebar" ? editingId : null}
          onEdit={(id) => {
            setEditingId(id);
            setEditingWhere(id ? "sidebar" : null);
          }}
          onOpenMenu={(id, rect) => setMenu({ id, rect })}
          onOpenSettings={() => setShowSettings(true)}
        />

        <section className="main">
          <TaskBar
            editingId={editingWhere === "bar" ? editingId : null}
            onEdit={(id) => {
              setEditingId(id);
              setEditingWhere(id ? "bar" : null);
            }}
          />
          <Transcript />
          <Composer
            onOpenPicker={(rect, onPick) => setPicker({ rect, onPick })}
            onOpenSettings={() => setShowSettings(true)}
          />
        </section>

        <ResultsPanel hidden={!showAside} onCollapse={() => setShowAside(false)} />
      </div>

      {picker && (
        <FilePicker
          rect={picker.rect}
          onPick={picker.onPick}
          onClose={() => setPicker(null)}
        />
      )}

      {menu && (
        <TaskMenu
          id={menu.id}
          rect={menu.rect}
          onClose={() => setMenu(null)}
          onRename={(id) => {
            if (st.search) store.setSearch("");
            setEditingId(id);
            // 菜单锚点在侧栏，但输入框放在顶栏更稳（侧栏可能已收起）。
            setEditingWhere("bar");
            if (id !== st.activeId) void store.activate(id);
          }}
        />
      )}

      {st.folderPick && (
        <FolderPicker key={st.folderPick.forTask || "new"} initial={st.folderPick.initial} />
      )}

      {showSettings && <SettingsPage onClose={() => setShowSettings(false)} />}

      {showShortcuts && <ShortcutsPanel onClose={() => setShowShortcuts(false)} />}

      {st.paletteOpen && (
        <CommandPalette
          onOpenSettings={() => setShowSettings(true)}
          onOpenShortcuts={() => setShowShortcuts(true)}
          onToggleSidebar={() => setShowSidebar((v) => !v)}
          onToggleAside={() => setShowAside((v) => !v)}
        />
      )}

      <Toast />
    </div>
  );
}

import { useLayoutEffect, useRef, useState } from "react";
import { resolvePersona, setTaskPersona, useAppState } from "../lib/store";
import { Icon } from "./Icon";

/**
 * 输入区的人设下拉：切换当前本机任务使用的系统提示词人设。
 *
 * 选项与「设置 → 人设」里的列表同源；锚点在输入区底栏，所以菜单向上弹出并在量到
 * 尺寸后再定位，避免贴边时闪一下。
 */
export function PersonaMenu({
  id,
  rect,
  onClose,
}: {
  id: string;
  rect: DOMRect;
  onClose: () => void;
}) {
  const st = useAppState();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos({
      left: Math.max(10, Math.min(rect.left, window.innerWidth - el.offsetWidth - 10)),
      top: Math.max(10, rect.top - el.offsetHeight - 6),
    });
  }, [rect]);

  const task = st.tasks.find((t) => t.id === id) ?? null;
  const currentId = resolvePersona(st.settings, task?.persona)?.id || "";
  const list = st.settings?.personas || [];

  return (
    <div
      className="menu show personamenu"
      data-menu-pop
      ref={ref}
      style={{
        left: pos?.left ?? rect.left,
        top: pos?.top ?? rect.top,
        visibility: pos ? "visible" : "hidden",
      }}
    >
      <div className="menu-label">使用的人设</div>
      {list.length === 0 && <div className="menu-empty">还没有配置人设，请先到设置里添加</div>}
      {list.map((p) => (
        <button
          key={p.id}
          title={p.prompt.trim() ? p.prompt : "内置提示词，不附加任何额外指令"}
          className={p.id === currentId ? "on" : ""}
          onClick={() => {
            onClose();
            if (p.id !== currentId) void setTaskPersona(id, p.id);
          }}
        >
          <Icon name={p.id === currentId ? "check" : "user"} />
          <span className="mn">{p.name || "未命名人设"}</span>
          {p.prompt.trim() ? <span className="mm">自定义</span> : <span className="mm">默认</span>}
        </button>
      ))}
    </div>
  );
}

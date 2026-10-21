import { useLayoutEffect, useRef, useState } from "react";
import { resolveModel, setTaskModel, useAppState } from "../lib/store";
import { Icon } from "./Icon";

/**
 * 输入区的模型下拉：切换当前任务使用的模型档案。
 *
 * 选项与「设置 → 模型」里的列表同源；锚点在输入区底栏，所以菜单向上弹出并在
 * 量到尺寸后再定位，避免贴边时闪一下。
 */
export function ModelMenu({
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
  const currentId = resolveModel(st.settings, task?.model)?.id || "";
  const list = st.settings?.models || [];

  return (
    <div
      className="menu show modelmenu"
      data-menu-pop
      ref={ref}
      style={{
        left: pos?.left ?? rect.left,
        top: pos?.top ?? rect.top,
        visibility: pos ? "visible" : "hidden",
      }}
    >
      <div className="menu-label">使用的模型</div>
      {list.length === 0 && <div className="menu-empty">还没有配置模型，请先到设置里添加</div>}
      {list.map((m) => (
        <button
          key={m.id}
          title={m.model ? `模型：${m.model}` : undefined}
          className={m.id === currentId ? "on" : ""}
          onClick={() => {
            onClose();
            if (m.id !== currentId) void setTaskModel(id, m.id);
          }}
        >
          <Icon name={m.id === currentId ? "check" : "spark"} />
          <span className="mn">{m.name || m.model || m.id}</span>
          {m.model && m.name && m.model !== m.name ? <span className="mm">{m.model}</span> : null}
        </button>
      ))}
    </div>
  );
}

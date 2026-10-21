import { useLayoutEffect, useRef, useState } from "react";
import { insertSnippet, useAppState } from "../lib/store";
import { Icon } from "./Icon";

/**
 * 输入区的提示词片段下拉：点一段就把它拼到输入框末尾。
 *
 * 片段在「设置 → 提示词片段」里维护；这里只负责插入，不改变任何任务配置，所以
 * 本机/云端任务都能用。锚点在输入区底栏，菜单向上弹出并在量到尺寸后再定位。
 */
export function SnippetMenu({ rect, onClose }: { rect: DOMRect; onClose: () => void }) {
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

  const list = st.settings?.snippets || [];

  return (
    <div
      className="menu show snippetmenu"
      data-menu-pop
      ref={ref}
      style={{
        left: pos?.left ?? rect.left,
        top: pos?.top ?? rect.top,
        visibility: pos ? "visible" : "hidden",
      }}
    >
      <div className="menu-label">插入提示词片段</div>
      {list.length === 0 && (
        <div className="menu-empty">还没有片段，可在「设置 → 提示词片段」里添加</div>
      )}
      {list.map((s) => (
        <button
          key={s.id}
          title={s.text}
          onClick={() => {
            onClose();
            insertSnippet(s.text);
          }}
        >
          <Icon name="bolt" />
          <span className="mn">{s.name || "未命名片段"}</span>
        </button>
      ))}
    </div>
  );
}

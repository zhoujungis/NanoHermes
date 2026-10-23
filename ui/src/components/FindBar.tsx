import { useEffect, useRef } from "react";
import { Icon } from "./Icon";

/**
 * 会话内查找条（Ctrl+F）。浮在消息区右上角，输入即跳转。
 *
 * 计数由消息区算好传进来（`total` / `index` 都是从 0 起，index 为 -1 表示没有命中），
 * 这里只负责输入、上一个/下一个与关闭。
 */
export function FindBar({
  open,
  focusTick,
  query,
  index,
  total,
  onQuery,
  onStep,
  onClose,
}: {
  open: boolean;
  focusTick: number;
  query: string;
  index: number;
  total: number;
  onQuery: (q: string) => void;
  onStep: (delta: number) => void;
  onClose: () => void;
}) {
  const input = useRef<HTMLInputElement>(null);

  // 关闭查找后把焦点还给打开前的元素（通常是输入框），键盘用户不必从头 Tab。
  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    return () => {
      const active = document.activeElement;
      // 只有焦点还在查找条里（或已丢失）时才还原，避免抢回用户刚点的别处。
      if (!active || active === document.body || active.closest(".find")) {
        prev?.focus?.();
      }
    };
  }, [open]);

  // 打开或再次按 Ctrl+F 时把焦点交给输入框，并选中已有查询便于整体替换。
  useEffect(() => {
    if (!open) return;
    const el = input.current;
    if (!el) return;
    el.focus();
    el.select();
  }, [open, focusTick]);

  if (!open) return null;

  const label = !query.trim() ? "" : total ? `${index + 1} / ${total}` : "无结果";

  return (
    <div className="find" role="search">
      <Icon name="search" />
      <input
        ref={input}
        value={query}
        placeholder="在对话中查找"
        aria-label="在对话中查找"
        spellCheck={false}
        onChange={(e) => onQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onStep(e.shiftKey ? -1 : 1);
          } else if (e.key === "Escape") {
            // 查找条自己有焦点时直接关掉，不必走到顶层的 Esc 链。
            e.preventDefault();
            onClose();
          }
        }}
      />
      <span className={"fcount" + (query.trim() && !total ? " none" : "")}>{label}</span>
      <button
        className="tbtn"
        title="上一个（Shift+Enter）"
        aria-label="上一个匹配"
        disabled={!total}
        onClick={() => onStep(-1)}
      >
        <Icon name="arrowUp" />
      </button>
      <button
        className="tbtn"
        title="下一个（Enter）"
        aria-label="下一个匹配"
        disabled={!total}
        onClick={() => onStep(1)}
      >
        <Icon name="arrowDown" />
      </button>
      <button className="tbtn" title="关闭查找（Esc）" aria-label="关闭查找" onClick={onClose}>
        <Icon name="x" />
      </button>
    </div>
  );
}

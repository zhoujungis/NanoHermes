import { useEffect, useRef, useState } from "react";

/**
 * 可就地编辑的标题：未编辑时是普通文本（双击进入编辑），编辑时是输入框。
 * Enter 提交，Esc 还原，失焦提交。
 */
export function InlineEdit({
  value,
  editing,
  onStart,
  onDone,
  onCancel,
  className,
}: {
  value: string;
  editing: boolean;
  onStart: () => void;
  onDone: (next: string) => void;
  onCancel: () => void;
  className?: string;
}) {
  const [draft, setDraft] = useState(value);
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) return;
    setDraft(value);
    ref.current?.focus();
    ref.current?.select();
  }, [editing, value]);

  if (!editing) {
    return (
      <div
        className={className}
        onDoubleClick={(e) => {
          e.stopPropagation();
          onStart();
        }}
      >
        {value}
      </div>
    );
  }

  return (
    <input
      ref={ref}
      className="inline-input"
      value={draft}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          onDone(draft.trim());
        } else if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
      onBlur={() => onDone(draft.trim())}
      onClick={(e) => e.stopPropagation()}
    />
  );
}

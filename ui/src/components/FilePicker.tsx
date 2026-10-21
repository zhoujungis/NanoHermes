import { useEffect, useMemo, useRef, useState } from "react";
import { addRef } from "../lib/store";
import { errText, invoke } from "../lib/tauri";
import { Icon } from "./Icon";

const WIDTH = 380;
const MAX_ITEMS = 300;

/** 工作区文件选择器：搜索并作为引用加入本次提问。 */
export function FilePicker({
  rect,
  onPick,
  onClose,
}: {
  rect: DOMRect;
  /** 不传时选中即加入引用；传了则由调用方决定选中行为（如 @ 插入路径到输入框）。 */
  onPick?: (path: string) => void;
  onClose: () => void;
}) {
  const [files, setFiles] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    let alive = true;
    invoke<string[]>("list_workspace_files")
      .then((list) => {
        if (alive) setFiles(list);
      })
      .catch((e) => {
        if (!alive) return;
        setFiles([]);
        setError(errText(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  const items = useMemo(() => {
    const key = query.trim().toLowerCase();
    return (files ?? []).filter((f) => !key || f.toLowerCase().includes(key)).slice(0, MAX_ITEMS);
  }, [files, query]);

  const pick = (path: string) => {
    if (onPick) onPick(path);
    else addRef(path);
    onClose();
  };

  return (
    <div
      className="picker show"
      style={{
        left: Math.max(12, Math.min(rect.left, window.innerWidth - WIDTH - 12)),
        bottom: window.innerHeight - rect.top + 8,
        width: WIDTH,
      }}
    >
      <div className="ph">
        <input
          ref={inputRef}
          value={query}
          placeholder="搜索工作区文件…"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)
          }
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              const first = items[0];
              if (first) pick(first);
            }
          }}
        />
      </div>
      <div className="plist">
        {files === null && <div className="pv-msg">正在读取工作区…</div>}
        {error && <div className="pv-msg">{error}</div>}
        {files !== null && !error && items.length === 0 && (
          <div className="pv-msg">没有匹配的文件</div>
        )}
        {items.map((f) => (
          <button key={f} title={f} onClick={() => pick(f)}>
            <Icon name="file" />
            <span>{f}</span>
          </button>
        ))}
      </div>
      <div className="pfoot">
        {files === null
          ? "正在读取工作区…"
          : error
            ? "无法读取工作区"
            : files.length
              ? `共 ${files.length} 个文件 · 选择后作为引用加入提问`
              : "工作区为空"}
      </div>
    </div>
  );
}

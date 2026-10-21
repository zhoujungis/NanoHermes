import { useEffect, useRef } from "react";
import { SHORTCUTS } from "../lib/shortcuts";
import { Icon } from "./Icon";

/**
 * 快捷键提示面板（Ctrl+/）：把所有键盘操作收进一张随时可呼出的清单。
 *
 * 数据来自 lib/shortcuts 的单一清单；关闭走外层 Esc 链，这里对自身获得焦点的
 * 情况再兜一层，点遮罩也可关闭。
 */
export function ShortcutsPanel({ onClose }: { onClose: () => void }) {
  const close = useRef<HTMLButtonElement>(null);

  // 打开后焦点落在关闭按钮：键盘用户可以直接 Esc / 回车收起。
  useEffect(() => {
    close.current?.focus();
  }, []);

  return (
    <div className="cmd-back" onMouseDown={onClose}>
      <div
        className="sc"
        role="dialog"
        aria-label="键盘快捷键"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }}
      >
        <div className="sc-head">
          <Icon name="keyboard" />
          <span className="t">键盘快捷键</span>
          <span className="sp" />
          <span className="kbd">Esc</span>
          <button
            ref={close}
            className="tbtn"
            title="关闭（Esc）"
            aria-label="关闭快捷键面板"
            onClick={onClose}
          >
            <Icon name="x" />
          </button>
        </div>
        <div className="sc-list">
          {SHORTCUTS.map((g) => (
            <div className="sc-group" key={g.group}>
              <div className="cmd-group">{g.group}</div>
              {g.items.map((it) => (
                <div className="sc-row" key={g.group + it.desc}>
                  <span className="sc-desc">{it.desc}</span>
                  <span className="sc-keys">
                    {it.keys.map((k, i) => (
                      <span className="kbd" key={i}>
                        {k}
                      </span>
                    ))}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

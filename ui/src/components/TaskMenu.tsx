import { useLayoutEffect, useRef, useState } from "react";
import {
  activate,
  deleteTask,
  openFolderPick,
  resetCurrent,
  setTaskLocation,
  taskIsCloud,
  togglePin,
  useAppState,
} from "../lib/store";
import { Icon } from "./Icon";

/** 任务行右键/更多菜单。 */
export function TaskMenu({
  id,
  rect,
  onClose,
  onRename,
}: {
  id: string;
  rect: DOMRect;
  onClose: () => void;
  onRename: (id: string) => void;
}) {
  const st = useAppState();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // 菜单尺寸要等渲染后才能量到，先隐藏定位再显示，避免闪一下。
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos({
      left: Math.min(rect.left, window.innerWidth - el.offsetWidth - 10),
      top: Math.min(rect.bottom + 4, window.innerHeight - el.offsetHeight - 10),
    });
  }, [rect]);

  const run = (fn: () => void) => {
    onClose();
    fn();
  };

  const task = st.tasks.find((t) => t.id === id) ?? null;
  const cloud = taskIsCloud(task);

  return (
    <div
      className="menu show"
      ref={ref}
      style={{
        left: pos?.left ?? rect.left,
        top: pos?.top ?? rect.bottom + 4,
        visibility: pos ? "visible" : "hidden",
      }}
    >
      <button onClick={() => run(() => onRename(id))}>
        <Icon name="pencil" />
        <span>重命名</span>
      </button>
      <button onClick={() => run(() => togglePin(id))}>
        <Icon name="pin" />
        <span>{task?.pinned ? "取消置顶" : "置顶任务"}</span>
      </button>
      <button onClick={() => run(() => void setTaskLocation(id, cloud ? "local" : "cloud"))}>
        <Icon name={cloud ? "laptop" : "cloud"} />
        <span>{cloud ? "切换为本机执行" : "切换为云端执行"}</span>
      </button>
      {!cloud && (
        <button
          onClick={() =>
            run(() => openFolderPick({ forTask: id, initial: task?.workspace || "" }))
          }
        >
          <Icon name="folder" />
          <span>更改工作区文件夹…</span>
        </button>
      )}
      <button
        onClick={() =>
          run(() => {
            void (async () => {
              if (id !== st.activeId) await activate(id, true);
              await resetCurrent();
            })();
          })
        }
      >
        <Icon name="refresh" />
        <span>清空对话</span>
      </button>
      <div className="sep" />
      <button className="danger" onClick={() => run(() => void deleteTask(id))}>
        <Icon name="trash" />
        <span>删除任务</span>
      </button>
    </div>
  );
}

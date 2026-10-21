import { useMemo } from "react";
import { rowButton } from "../lib/a11y";
import {
  activeTask,
  activate,
  closeNewTask,
  createTask,
  isCloud,
  openFolderPick,
  openNewTask,
  renameTask,
  setSearch,
  taskIsCloud,
  useAppState,
  type Task,
} from "../lib/store";
import { Icon } from "./Icon";
import { InlineEdit } from "./InlineEdit";

const DAY = 86_400_000;

/** 任务按「今天 / 昨天 / 本周 / 更早」分组，和更新时间的直觉一致。 */
function bucketOf(ts: number, dayStart: number): string {
  if (ts >= dayStart) return "今天";
  if (ts >= dayStart - DAY) return "昨天";
  if (ts >= dayStart - 6 * DAY) return "本周";
  return "更早";
}

export function Sidebar({
  hidden,
  editingId,
  onEdit,
  onOpenMenu,
  onOpenSettings,
}: {
  hidden: boolean;
  editingId: string | null;
  onEdit: (id: string | null) => void;
  onOpenMenu: (id: string, rect: DOMRect) => void;
  onOpenSettings: () => void;
}) {
  const st = useAppState();
  const q = st.search.trim().toLowerCase();

  const groups = useMemo(() => {
    const items = st.tasks.filter((t) => {
      if (!q) return true;
      if ((t.title || "").toLowerCase().includes(q)) return true;
      return t.msgs.some((m) => (m.t || "").toLowerCase().includes(q));
    });
    items.sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0));

    const now = new Date();
    const dayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    const buckets: Array<[string, Task[]]> = [
      ["今天", []],
      ["昨天", []],
      ["本周", []],
      ["更早", []],
    ];
    const byLabel = new Map(buckets);
    // 置顶的任务单独成一组、固定排在最前，不再按更新日期散在各组里。
    const pinned: Task[] = [];
    items.forEach((t) => {
      if (t.pinned) pinned.push(t);
      else byLabel.get(bucketOf(t.updatedAt || t.createdAt || 0, dayStart))?.push(t);
    });
    const out: Array<[string, Task[]]> = pinned.length ? [["置顶", pinned]] : [];
    return {
      total: items.length,
      buckets: out.concat(buckets.filter(([, list]) => list.length > 0)),
    };
  }, [st.tasks, q]);

  // 执行位置属于任务：底部显示当前任务的真实位置（没有选中任务时退回默认位置）。
  const cur = activeTask();
  const cloud = cur ? taskIsCloud(cur) : isCloud(st.settings);
  const ws = (st.settings?.workspace || "").trim();
  const cloudUrl = (st.settings?.cloud_url || "").trim();
  const place = cloud ? cloudUrl : ws;
  const avatar = cloud
    ? "云"
    : ws.replace(/[\\/]+$/, "").split(/[\\/]/).pop()?.slice(0, 1).toUpperCase() || "N";

  return (
    <aside className={"sidebar" + (hidden ? " collapsed" : "")}>
      <div className="sb-top">
        <button className="newtask" onClick={() => (st.newTaskOpen ? closeNewTask() : openNewTask())}>
          <Icon name="plus" />
          <span>新建任务</span>
          <span className="k">Ctrl N</span>
        </button>

        {st.newTaskOpen && (
          <div className="newpick">
            <button
              onClick={() => {
                closeNewTask();
                openFolderPick({ forTask: null, initial: ws });
              }}
            >
              <Icon name="laptop" />
              <span className="w">
                <b>本机执行 · 选文件夹</b>
                <em title={ws}>{ws || "点击选择工作区目录"}</em>
              </span>
            </button>
            <button onClick={() => void createTask(false, "cloud")}>
              <Icon name="cloud" />
              <span className="w">
                <b>云端执行</b>
                <em title={cloudUrl}>{cloudUrl || "未配置云服务地址"}</em>
              </span>
            </button>
          </div>
        )}

        <div className="search">
          <Icon name="search" />
          <input
            value={st.search}
            placeholder="搜索任务"
            spellCheck={false}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
      </div>

      <div className="sb-scroll">
        {groups.total === 0 ? (
          <div className="sb-empty">
            {q ? "没有匹配的任务" : "还没有任务，点击上方新建。"}
          </div>
        ) : (
          groups.buckets.map(([label, list]) => (
            <div key={label}>
              <div className="group-label">{label}</div>
              {list.map((t) => (
                <div
                  key={t.id}
                  className={"task" + (t.id === st.activeId ? " active" : "")}
                  {...rowButton(() => void activate(t.id))}
                  onClick={() => void activate(t.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onOpenMenu(t.id, e.currentTarget.getBoundingClientRect());
                  }}
                >
                  <InlineEdit
                    value={t.title || "新任务"}
                    editing={editingId === t.id && st.activeId !== t.id}
                    onStart={() => onEdit(t.id)}
                    onDone={(v) => {
                      renameTask(t.id, v);
                      onEdit(null);
                    }}
                    onCancel={() => onEdit(null)}
                    className="t"
                  />
                  {t.pinned && <Icon name="pin" className="tpin" />}
                  {taskIsCloud(t) && (
                    <Icon name="cloud" className="tloc" />
                  )}
                  <button
                    className="more"
                    data-menu-anchor
                    title="更多"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpenMenu(t.id, e.currentTarget.getBoundingClientRect());
                    }}
                  >
                    <Icon name="dot3" />
                  </button>
                </div>
              ))}
            </div>
          ))
        )}
      </div>

      <div className="sb-foot">
        <div className="avatar">{avatar}</div>
        <div className="who">
          <div className="l1">{cloud ? "云端服务" : "本机工作区"}</div>
          <div className="l2" title={place || "未设置"}>
            {place || (cloud ? "未配置云服务地址" : "未设置")}
          </div>
        </div>
        <button className="tbtn" title="设置" onClick={onOpenSettings}>
          <Icon name="gear" />
        </button>
      </div>
    </aside>
  );
}

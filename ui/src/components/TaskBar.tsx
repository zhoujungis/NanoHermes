import { activeTask, exportCurrent, renameTask, resetCurrent, useAppState } from "../lib/store";
import { Icon } from "./Icon";
import { InlineEdit } from "./InlineEdit";

/** 用量数字：小于一千直接显示，超过则折算成 k，避免长数字挤压标题。 */
function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  return (n / 1000).toFixed(n < 10000 ? 1 : 0) + "k";
}

/** 对话区顶部：任务标题、运行状态、用量、导出与清空上下文。 */
export function TaskBar({
  editingId,
  onEdit,
}: {
  editingId: string | null;
  onEdit: (id: string | null) => void;
}) {
  const st = useAppState();
  const task = activeTask();
  const usage = task?.usage;

  return (
    <div className="taskbar">
      <InlineEdit
        value={task?.title || "新任务"}
        editing={!!task && editingId === task.id}
        onStart={() => task && onEdit(task.id)}
        onDone={(v) => {
          if (task) renameTask(task.id, v);
          onEdit(null);
        }}
        onCancel={() => onEdit(null)}
        className="title"
      />
      <div
        className={
          "state-chip" + (st.busy ? " run" : "") + (st.stopping ? " stopping" : "")
        }
        title={st.stopping ? "已请求停止，正在等待当前步骤结束" : undefined}
      >
        <span className="dot" />
        <span>{st.stopping ? "停止中" : st.busy ? "执行中" : "就绪"}</span>
      </div>
      {usage && usage.requests > 0 && (
        <div
          className="usage"
          title={`本任务累计：${usage.requests} 次请求 · 输入 ${usage.prompt_tokens} · 输出 ${usage.completion_tokens} tokens`}
        >
          <Icon name="spark" />
          <span>{formatTokens(usage.prompt_tokens + usage.completion_tokens)} tokens</span>
        </div>
      )}
      <div className="fill" data-tauri-drag-region />
      <button className="tbtn" title="导出对话为 Markdown" onClick={() => void exportCurrent()}>
        <Icon name="download" />
      </button>
      <button className="tbtn" title="重命名" onClick={() => task && onEdit(task.id)}>
        <Icon name="pencil" />
      </button>
      <button className="tbtn" title="清空对话上下文" onClick={() => void resetCurrent()}>
        <Icon name="refresh" />
      </button>
    </div>
  );
}

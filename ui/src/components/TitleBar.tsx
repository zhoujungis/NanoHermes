import { appWindow } from "../lib/tauri";
import {
  activeTask,
  isCloud,
  modelLabel,
  resolveModel,
  taskIsCloud,
  useAppState,
} from "../lib/store";
import { BrandMark, Icon } from "./Icon";

/** 顶栏：品牌、当前模型/工作区、面板开关、设置、窗口控制。 */
export function TitleBar({
  showSidebar,
  showAside,
  onToggleSidebar,
  onToggleAside,
  onOpenSettings,
}: {
  showSidebar: boolean;
  showAside: boolean;
  onToggleSidebar: () => void;
  onToggleAside: () => void;
  onOpenSettings: () => void;
}) {
  const st = useAppState();
  const settings = st.settings;
  const win = appWindow();
  const task = activeTask();

  // 执行位置跟着当前任务走；没有任务时才退回设置里的默认位置。用默认值会让
  // 一个本机任务显示「云端执行」、或反过来提示未配置 Key。
  const cloud = task ? taskIsCloud(task) : isCloud(settings);
  // 本机任务用「它自己选的模型档案」，没有任务时才退回设置里的默认档案。
  const profile = resolveModel(settings, task?.model);
  const hasKey = !!(profile?.api_key || "").trim();
  const cloudUrl = (settings?.cloud_url || "").trim();
  const ready = cloud ? !!cloudUrl : hasKey;
  const model = cloud ? "云端执行" : modelLabel(settings, task?.model);
  // 工作区也按当前任务取：顶栏显示的就是这个任务真正会读写的目录。
  const ws = (task?.workspace || settings?.workspace || "").trim();
  const chipText = ready ? (cloud ? "云端执行" : model) : cloud ? "未配置云服务" : "未配置模型服务";
  const chipTitle = cloud
    ? "执行位置：云端\n地址：" +
      (cloudUrl || "（未填写）") +
      "\n模型与工作区由远端 daemon 的启动参数决定"
    : `模型：${model}` +
      (profile?.model ? `（${profile.model}）` : "") +
      (ws ? `\n工作区：${ws}` : "") +
      (hasKey ? "" : "\n尚未填写 API Key");

  return (
    <header
      className="titlebar"
      onDoubleClick={(e) => {
        const t = e.target as HTMLElement;
        if (t.closest("button") || t.closest(".tb-chip")) return;
        void win?.toggleMaximize();
      }}
    >
      <div className="brand" data-tauri-drag-region>
        <div className="mark" aria-hidden="true">
          <BrandMark size={20} />
        </div>
        <div className="name">NanoHermes</div>
        <div className="tag">AGENT</div>
      </div>
      <div className="fill" data-tauri-drag-region />

      <button
        type="button"
        className="tb-chip"
        title={chipTitle}
        aria-label={`执行位置：${chipText}。打开设置`}
        onClick={onOpenSettings}
      >
        <span className={"dot" + (ready ? "" : " off")} />
        <span>{chipText}</span>
      </button>
      <div className="tb-sep" />

      <button
        type="button"
        className={"tbtn" + (showSidebar ? " on" : "")}
        title="任务栏（Ctrl+B）"
        aria-pressed={showSidebar}
        onClick={onToggleSidebar}
      >
        <Icon name="panelLeft" />
      </button>
      <button
        type="button"
        className={"tbtn" + (showAside ? " on" : "")}
        title="结果区（Ctrl+J）"
        aria-pressed={showAside}
        onClick={onToggleAside}
      >
        <Icon name="panelRight" />
      </button>
      <button className="tbtn" title="设置（Ctrl+,）" onClick={onOpenSettings}>
        <Icon name="gear" />
      </button>

      <div className="wins">
        <button className="tbtn" title="最小化" onClick={() => void win?.minimize()}>
          <Icon name="minus" />
        </button>
        <button className="tbtn" title="最大化" onClick={() => void win?.toggleMaximize()}>
          <Icon name="square" />
        </button>
        <button className="tbtn close" title="关闭" onClick={() => void win?.close()}>
          <Icon name="x" />
        </button>
      </div>
    </header>
  );
}

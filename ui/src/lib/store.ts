/**
 * 应用状态与所有副作用动作。
 *
 * 采用「外部 store + useSyncExternalStore」而不是 useReducer：流式增量、Tauri
 * 事件回调、异步命令都需要在 React 之外读写同一份状态，外部 store 能避免在
 * effect 里再同步一次。
 */
import { useSyncExternalStore } from "react";
import { errText, invoke, listen } from "./tauri";
import { formatBytes, MAX_ATTACHMENTS, toAttachment, type Attachment } from "./image";

const LS_KEY = "nanohermes.state.v1";
const MAX_MSGS = 200;
const MAX_TASKS = 120;
const MAX_FILES = 200;

const EVENT_DELTA = "nanohermes://delta";
const EVENT_TOOL = "nanohermes://tool";
const EVENT_NOTE = "nanohermes://note";
const EVENT_USAGE = "nanohermes://usage";

/** 结果区里文件的归类：修改 / 读取。 */
export type Kind = "w" | "r";
export type Filter = "all" | Kind;

/** 结果区的两个页签：工作区文件树 / 本任务产物。 */
export type PanelTab = "tree" | "artifacts";

export interface Msg {
  k: "user" | "ai" | "tool" | "note";
  /** 文本内容（user / ai / note）。 */
  t?: string;
  /** tool 事件的工具名与参数摘要。 */
  name?: string;
  summary?: string;
  /** tool 事件的工具结果（可能被截断），用于展开查看细节。 */
  out?: string;
  /** 随该条消息发出的图片（data URL）。 */
  imgs?: string[];
  ts: number;
  /** 仍在流式输出中。 */
  streaming?: boolean;
  /** 该条为错误信息。 */
  err?: boolean;
  /** 长回答被折叠，只显示两行摘要。 */
  fold?: boolean;
}

/** 一条排队等待发送的消息（当前回答结束后自动发出）。 */
export interface QueueItem {
  text: string;
  imgs?: string[];
  /** 发送时要内联进提示词的工作区引用。 */
  refs: string[];
}

export interface Touched {
  path: string;
  kind: Kind;
  ts: number;
}

/** 一个任务累计的 token 用量（本机与云端都从后端的增量累加而来）。 */
export interface Usage {
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
}

export interface Task {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  msgs: Msg[];
  files: Touched[];
  /** 该任务的执行位置：`local` 本机进程内，`cloud` 远程 daemon。 */
  location: string;
  /** 该任务的本机工作区文件夹（云端任务不使用）。 */
  workspace: string;
  /** 该任务使用哪个模型档案（settings.models 里的 id）；空串表示跟随默认。 */
  model: string;
  /** 该任务使用哪个人设（settings.personas 里的 id）；空串表示跟随默认。 */
  persona: string;
  /** 置顶的任务排在列表最前，且不参与日期分组。 */
  pinned?: boolean;
  /** 累计用量，未运行过则没有。 */
  usage?: Usage;
  /** 运行中继续发送的消息，等本轮结束后按顺序发出。 */
  queue?: QueueItem[];
}

/** 请求打开文件夹选择器；`forTask` 为空表示「给新建的本机任务选目录」。 */
export interface FolderPick {
  forTask: string | null;
  initial: string;
}

/** 一个命名的模型服务：设置里可保存多个，输入区可以随时切换。 */
export interface ModelProfile {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  model: string;
}

/** 一套附加到内置系统提示词的人设；prompt 为空表示「不加任何附加指令」。 */
export interface Persona {
  id: string;
  name: string;
  prompt: string;
}

/** 一段可复用的提示词片段，输入区一键插入。 */
export interface Snippet {
  id: string;
  name: string;
  text: string;
}

export interface Settings {
  /** 旧版单模型三元组：仍原样保留，避免老配置文件丢数据。 */
  api_key: string;
  base_url: string;
  model: string;
  /** 可选模型列表；后端迁移后至少有一项。 */
  models: ModelProfile[];
  /** 新任务默认使用的模型档案 id，也是任务未指定时的兜底。 */
  active_model: string;
  /** 可选人设列表；后端迁移后至少有一项（默认人设不带任何附加指令）。 */
  personas: Persona[];
  /** 新任务默认使用的人设 id，也是任务未指定时的兜底。 */
  active_persona: string;
  /** 可复用的提示词片段。 */
  snippets: Snippet[];
  workspace: string;
  mode: string;
  /** 执行位置：`local` 进程内运行，`cloud` 转发到远程 NanoHermes daemon。 */
  location: string;
  /** 云服务基地址，例如 http://203.0.113.7:8787 */
  cloud_url: string;
  /** 云服务的 Bearer Token（daemon 的 NANOHERMES_DAEMON_TOKEN）。 */
  cloud_token: string;
  /** 高级运行参数，仅对本机任务生效（云端行为由远端 daemon 决定）。 */
  thinking: boolean;
  project_docs: boolean;
  /** 长期记忆：对话收尾自动沉淀经验/偏好，新会话按相关度检索注入。 */
  memory: boolean;
  temperature: number;
  max_iterations: number;
  max_tokens: number;
}

/**
 * 某个任务/设置实际用到的模型档案：给定了 id 就用它，否则用默认档案，再否则用
 * 列表里的第一个。列表为空时返回 null（后端此时会回落到旧三元组）。
 */
export function resolveModel(s: Settings | null, id?: string | null): ModelProfile | null {
  if (!s) return null;
  const want = (id ?? "").trim();
  const list = s.models || [];
  return (
    list.find((m) => m.id === want) ??
    list.find((m) => m.id === s.active_model) ??
    list[0] ??
    null
  );
}

/** 档案的展示名：优先用用户起的名字，退回模型名，最后退回「默认模型」。 */
export function modelLabel(s: Settings | null, id?: string | null): string {
  const p = resolveModel(s, id);
  return (p?.name || p?.model || "").trim() || "默认模型";
}

/**
 * 某个任务/设置实际用到的人设：给定了 id 就用它，否则用默认人设，再否则用列表
 * 里的第一个。列表为空时返回 null（后端此时不会附加任何指令）。
 */
export function resolvePersona(s: Settings | null, id?: string | null): Persona | null {
  if (!s) return null;
  const want = (id ?? "").trim();
  const list = s.personas || [];
  return (
    list.find((p) => p.id === want) ??
    list.find((p) => p.id === s.active_persona) ??
    list[0] ??
    null
  );
}

/** 人设的展示名：优先用用户起的名字，最后退回「默认人设」。 */
export function personaLabel(s: Settings | null, id?: string | null): string {
  const p = resolvePersona(s, id);
  return (p?.name || "").trim() || "默认人设";
}

/** 该人设是否真的会改变模型行为（有非空附加指令）。 */
export function personaIsCustom(p: Persona | null): boolean {
  return !!p && !!p.prompt.trim();
}

/** 设置的落盘状态：输入即保存，这里只用来给用户一个明确的反馈。 */
export type SettingsSync = "idle" | "pending" | "saving" | "saved" | "error";

/** 提示条的三个等级：普通说明 / 操作成功 / 出错。 */
export type ToastKind = "info" | "ok" | "err";

/** 新任务的默认执行位置是否为云端。 */
export const isCloud = (s: Settings | null): boolean =>
  (s?.location || "local") === "cloud";

/** 该任务是否在云端执行。 */
export const taskIsCloud = (t: Task | null): boolean =>
  (t?.location || "local") === "cloud";

export interface Preview {
  path: string;
  text: string | null;
  loading: boolean;
  error: string | null;
}

export interface State {
  tasks: Task[];
  activeId: string | null;
  busy: boolean;
  /** 已向后端发出停止请求，正在等当前步骤收尾。 */
  stopping: boolean;
  settings: Settings | null;
  mode: string;
  /** 输入框内容。 */
  prompt: string;
  /** 本次提问引用的工作区文件。 */
  refs: string[];
  /** 已附加、等待随下一条消息发出的图片。 */
  attachments: Attachment[];
  /** 递增信号：请求把键盘焦点交回输入框。 */
  focusTick: number;
  filter: Filter;
  /** 结果区当前页签。 */
  panelTab: PanelTab;
  search: string;
  preview: Preview | null;
  toast: string;
  /** 当前提示条的等级，决定配色与图标。 */
  toastKind: ToastKind;
  /** 设置是否已落盘。 */
  settingsSync: SettingsSync;
  /** 最近一次设置保存失败的原因。 */
  settingsSyncError: string;
  /** 新建任务时展开的「本机 / 云端」选择面板。 */
  newTaskOpen: boolean;
  /** 正在打开的文件夹选择器。 */
  folderPick: FolderPick | null;
  /** Ctrl+K 命令面板是否打开。 */
  paletteOpen: boolean;
  /** Ctrl+F 会话内查找条是否打开。 */
  findOpen: boolean;
  /** 递增信号：请求把键盘焦点交给查找输入框（Ctrl+F 已打开时再次按也要生效）。 */
  findTick: number;
}

let state: State = {
  tasks: [],
  activeId: null,
  busy: false,
  stopping: false,
  settings: null,
  mode: "default",
  prompt: "",
  refs: [],
  attachments: [],
  focusTick: 0,
  filter: "all",
  panelTab: "artifacts",
  search: "",
  preview: null,
  toast: "",
  toastKind: "info",
  settingsSync: "idle",
  settingsSyncError: "",
  newTaskOpen: false,
  folderPick: null,
  paletteOpen: false,
  findOpen: false,
  findTick: 0,
};

const listeners = new Set<() => void>();

export function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export const getState = (): State => state;

function commit(patch: Partial<State>): void {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

export function useAppState(): State {
  return useSyncExternalStore(subscribe, getState, getState);
}

/* ------------------------------------------------------------------ 持久化 */

let saveTimer: number | undefined;
/** 流式期间攒下的待落盘变更（见 scheduleSave）。 */
let savePending = false;

function scheduleSave(): void {
  // 流式期间每一帧都在改消息，而 save() 要序列化整份状态——其中含图片 data URL，
  // 单张就有几百 KB。每 300ms 做一次会把主线程按住，鼠标都会卡住，所以流式期间
  // 只打一个标记，等这一轮结束（closeStream / 收尾）再统一落盘。
  if (streamIndex >= 0) {
    savePending = true;
    return;
  }
  if (saveTimer !== undefined) return;
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined;
    save();
  }, 300);
}

/** 把流式期间攒下的变更真正写一次盘。 */
function flushSave(): void {
  if (!savePending) return;
  savePending = false;
  save();
}

/** 待写入 localStorage 的精简快照；`dropImages` 用于配额不足时的降级重试。 */
function snapshot(dropImages: boolean) {
  // 流式期间真正的文本在独立通道里，状态中的气泡还是 `t: ""`。直接落盘会存下
  // 一个 `streaming: true` 的空气泡——下次启动时它永远转圈，而且没有终态文本可
  // 以恢复。落盘前把实时文本合进去，并清掉所有 streaming 标记。
  const liveId = streamIndex >= 0 ? runTaskId ?? state.activeId : null;
  const liveIndex = liveId ? streamIndex : -1;
  const liveText = streamText + pendingDelta;

  return {
    activeId: state.activeId,
    tasks: state.tasks.slice(-MAX_TASKS).map((t) => ({
      id: t.id,
      title: t.title,
      createdAt: t.createdAt,
      updatedAt: t.updatedAt,
      msgs: t.msgs
        .map((m, i) => {
          const flushed =
            t.id === liveId && i === liveIndex && m.k === "ai" && liveText.trim()
              ? liveText
              : m.t;
          const next = m.streaming ? { ...m, t: flushed, streaming: false } : m;
          return dropImages && next.imgs?.length ? { ...next, imgs: undefined } : next;
        })
        // 空的 AI 气泡没有意义（模型先调用工具是常态），不要持久化。
        .filter((m) => !(m.k === "ai" && !m.t && !m.imgs?.length))
        .slice(-MAX_MSGS),
      files: t.files.slice(-MAX_FILES),
      location: t.location || "local",
      workspace: t.workspace || "",
      model: t.model || "",
      persona: t.persona || "",
      pinned: !!t.pinned,
      usage: t.usage,
      queue: (t.queue || []).map((q) =>
        dropImages && q.imgs?.length ? { ...q, imgs: undefined } : q,
      ),
    })),
  };
}

function save(): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(snapshot(false)));
  } catch {
    // 图片是 data URL，一张几百 KB。配额不够时宁可丢掉历史里的图片，也要保住
    // 任务与对话记录本身（否则整个状态都存不下来）。
    try {
      localStorage.setItem(LS_KEY, JSON.stringify(snapshot(true)));
    } catch {
      /* 本地存储不可用（隐私模式等）时静默降级 */
    }
  }
}

function load(): void {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return;
    const data = JSON.parse(raw) as { activeId?: string; tasks?: Partial<Task>[] };
    const tasks: Task[] = (data.tasks || [])
      .filter((t): t is Partial<Task> & { id: string } => !!t && typeof t.id === "string")
      .map((t) => ({
        id: t.id,
        title: t.title || "新任务",
        createdAt: t.createdAt || Date.now(),
        updatedAt: t.updatedAt || t.createdAt || Date.now(),
        // 旧版本可能在流式中途落盘，留下 `streaming: true` 的空气泡；清掉标记，
        // 避免加载后永远转圈。
        msgs: Array.isArray(t.msgs)
          ? t.msgs
              .map((m) => (m?.streaming ? { ...m, streaming: false } : m))
              .filter((m) => !(m?.k === "ai" && !m.t && !m.imgs?.length))
          : [],
        files: Array.isArray(t.files) ? t.files : [],
        location: t.location === "cloud" ? "cloud" : "local",
        workspace: typeof t.workspace === "string" ? t.workspace : "",
        model: typeof t.model === "string" ? t.model : "",
        persona: typeof t.persona === "string" ? t.persona : "",
        pinned: !!t.pinned,
        usage: t.usage && typeof t.usage.requests === "number" ? t.usage : undefined,
        queue: Array.isArray(t.queue)
          ? t.queue.filter(
              (q): q is QueueItem =>
                !!q && typeof q.text === "string" && Array.isArray(q.refs),
            )
          : undefined,
      }));
    state = { ...state, tasks, activeId: data.activeId || null };
  } catch {
    /* 数据损坏时从空状态启动 */
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => save());
}

/* ------------------------------------------------------------------- 工具 */

export function activeTask(): Task | null {
  return state.tasks.find((t) => t.id === state.activeId) ?? null;
}

/**
 * 正在跑的这一轮属于哪个任务。流式增量、工具行、用量都按它写回，而不是按「当前
 * 打开的任务」——否则用户在运行中切到别的任务，回答会写进另一个任务的对话里。
 * 没有正在跑的轮次时为 null，此时退回当前任务。
 */
let runTaskId: string | null = null;

function runTask(): Task | null {
  return runTaskId ? (state.tasks.find((t) => t.id === runTaskId) ?? null) : activeTask();
}

function replaceTask(id: string, patch: Partial<Task>): Task[] {
  return state.tasks.map((t) => (t.id === id ? { ...t, ...patch } : t));
}

function newId(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    "t" +
    d.getFullYear() +
    pad(d.getMonth() + 1) +
    pad(d.getDate()) +
    pad(d.getHours()) +
    pad(d.getMinutes()) +
    pad(d.getSeconds()) +
    Math.random().toString(36).slice(2, 6)
  );
}

export function titleFrom(text: string): string {
  const t = (text || "").replace(/\s+/g, " ").trim();
  if (!t) return "新任务";
  return t.length > 22 ? t.slice(0, 22) + "…" : t;
}

let toastTimer: number | undefined;

/**
 * 弹一条提示。`kind` 决定图标与配色（默认只是说明；成功动作用 ok，失败用 err），
 * 出错时多留一会儿，给用户看清原因的时间。
 */
export function toast(text: string, kind: ToastKind = "info"): void {
  commit({ toast: text, toastKind: kind });
  if (toastTimer !== undefined) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastTimer = undefined;
    commit({ toast: "" });
  }, kind === "err" ? 3200 : 2000);
}

/* -------------------------------------------------------------- 消息读写 */

// 导出供端到端测试造会话数据使用；运行期仍只在事件处理里调用。
export function pushMsg(m: Msg): number {
  const task = runTask();
  if (!task) return -1;
  let msgs = [...task.msgs, m];
  if (msgs.length > MAX_MSGS) msgs = msgs.slice(-MAX_MSGS);
  commit({ tasks: replaceTask(task.id, { msgs, updatedAt: Date.now() }) });
  scheduleSave();
  return msgs.length - 1;
}

function setMsg(index: number, patch: Partial<Msg>): void {
  const task = runTask();
  if (!task || index < 0 || index >= task.msgs.length) return;
  const msgs = task.msgs.slice();
  msgs[index] = { ...msgs[index], ...patch };
  commit({ tasks: replaceTask(task.id, { msgs, updatedAt: Date.now() }) });
  scheduleSave();
}

function dropMsg(index: number): void {
  const task = runTask();
  if (!task || index < 0 || index >= task.msgs.length) return;
  const msgs = task.msgs.slice();
  msgs.splice(index, 1);
  commit({ tasks: replaceTask(task.id, { msgs, updatedAt: Date.now() }) });
  scheduleSave();
}

/* ------------------------------------------------------------ 输入区状态 */

export const setPrompt = (prompt: string): void => commit({ prompt });
export const setSearch = (search: string): void => commit({ search });
export const setFilter = (filter: Filter): void => commit({ filter });
export const setPanelTab = (panelTab: PanelTab): void => commit({ panelTab });

export function addRef(path: string): void {
  if (state.refs.includes(path)) return;
  commit({ refs: [...state.refs, path] });
}

export function removeRef(path: string): void {
  commit({ refs: state.refs.filter((p) => p !== path) });
}

/** 把一段文本插到输入框的指定位置（@ 文件引用用），并把焦点交回输入框。 */
export function insertAt(pos: number, text: string): void {
  const p = state.prompt;
  const i = Math.max(0, Math.min(pos, p.length));
  commit({ prompt: p.slice(0, i) + text + p.slice(i), focusTick: state.focusTick + 1 });
}

/* -------------------------------------------------------------- 图片附件 */

/** 附加一张图片（粘贴、拖拽或选择文件都会走这里）。 */
export async function addAttachment(file: File): Promise<void> {
  if (state.attachments.length >= MAX_ATTACHMENTS) {
    toast(`一次最多附加 ${MAX_ATTACHMENTS} 张图片`);
    return;
  }
  try {
    const a = await toAttachment(file);
    commit({ attachments: [...state.attachments, a] });
    toast(`已附加 ${a.name}（${a.w}×${a.h} · ${formatBytes(a.bytes)}）`, "ok");
  } catch (e) {
    toast(errText(e), "err");
  }
}

export function removeAttachment(url: string): void {
  commit({ attachments: state.attachments.filter((a) => a.url !== url) });
}

export function clearAttachments(): void {
  if (state.attachments.length) commit({ attachments: [] });
}

/* -------------------------------------------------------------- 输入历史 */

const HISTORY_MAX = 50;
/** 发送过的提问，供输入框 ↑↓ 翻阅。只在本次会话内保留，不做持久化。 */
const history: string[] = [];

export function pushHistory(text: string): void {
  const t = text.trim();
  if (!t) return;
  // 连续重复的提问不重复入栈（反复追问同一句时历史保持干净）。
  if (history[history.length - 1] === t) return;
  history.push(t);
  if (history.length > HISTORY_MAX) history.shift();
}

export const inputHistory = (): readonly string[] => history;

/* ------------------------------------------------------------------ 任务 */

/** 展开新建任务时的「本机 / 云端」选择面板。 */
export const openNewTask = (): void => commit({ newTaskOpen: true });
export const closeNewTask = (): void => {
  if (state.newTaskOpen) commit({ newTaskOpen: false });
};

/** Ctrl+K 命令面板开合。 */
export const openPalette = (): void => commit({ paletteOpen: true });
export const closePalette = (): void => {
  if (state.paletteOpen) commit({ paletteOpen: false });
};

/** Ctrl+F 会话内查找条：打开时顺带把焦点交回输入框（findTick 是给组件的信号）。 */
export const openFind = (): void =>
  commit({ findOpen: true, findTick: state.findTick + 1 });
export const closeFind = (): void => {
  if (state.findOpen) commit({ findOpen: false });
};

/**
 * 新建任务。`location` 决定这个任务在哪里执行（不传则用设置里的默认位置），
 * 每个任务各选各的，互不影响。
 */
export async function createTask(
  silent = false,
  location?: string,
  workspace?: string,
): Promise<void> {
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  const where = location || state.settings?.location || "local";
  const ws = (workspace ?? state.settings?.workspace ?? "").trim();
  // 新任务固定用当前的默认模型档案：此后改设置里的默认项不会悄悄改掉老任务。
  const model = state.settings?.active_model || "";
  // 人设同理：新任务绑定当前默认人设，之后改默认值不会影响已有任务。
  const persona = state.settings?.active_persona || "";
  const now = Date.now();
  const task: Task = {
    id: newId(),
    title: "新任务",
    createdAt: now,
    updatedAt: now,
    msgs: [],
    files: [],
    location: where === "cloud" ? "cloud" : "local",
    workspace: ws,
    model,
    persona,
  };
  commit({ tasks: [...state.tasks, task], newTaskOpen: false });
  if (task.location === "local" && !ws) {
    // 本机任务必须有工作区，先把选择器摆出来（首次启动、删除任务后的兜底路径）。
    commit({ folderPick: { forTask: task.id, initial: "" } });
  }
  if (!silent) toast(where === "cloud" ? "已新建云端任务" : "已新建本机任务", "ok");
  await activate(task.id);
}

/** 让某个任务重新绑定：会话文件 + 它自己的工作区 + 它选的模型档案。 */
async function rebind(id: string): Promise<void> {
  const task = state.tasks.find((t) => t.id === id);
  try {
    await invoke("open_task", {
      id,
      workspace: task?.workspace || null,
      model: task?.model || null,
      persona: task?.persona || null,
    });
  } catch (e) {
    // 不要静默吞掉：界面已经显示新选择，后端若仍绑着旧配置，用户只会看到
    // 「模型/文件夹明明换了却不生效」。构建失败（缺密钥等）会在发送时单独报错，
    // 这里只报真正没能重绑的情况。
    toast("切换任务配置失败：" + errText(e), "err");
  }
}

/** 切换某个任务（含正在进行的任务）的执行位置。 */
export async function setTaskLocation(id: string, location: string): Promise<void> {
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  const where = location === "cloud" ? "cloud" : "local";
  commit({ tasks: state.tasks.map((t) => (t.id === id ? { ...t, location: where } : t)) });
  if (state.activeId === id) await rebind(id);
  scheduleSave();
  toast(where === "cloud" ? "已切换为云端执行" : "已切换为本机执行", "ok");
}

/** 更改某个任务的本地工作区文件夹。 */
export async function setTaskWorkspace(id: string, workspace: string): Promise<void> {
  const ws = workspace.trim();
  if (!ws) return;
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  commit({ tasks: state.tasks.map((t) => (t.id === id ? { ...t, workspace: ws } : t)) });
  if (state.activeId === id) await rebind(id);
  scheduleSave();
  toast("已切换工作区：" + ws, "ok");
}

/** 切换某个本机任务使用的模型档案（输入区的模型下拉与命令面板都走这里）。 */
export async function setTaskModel(id: string, model: string): Promise<void> {
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  const task = state.tasks.find((t) => t.id === id);
  if (!task) return;
  commit({ tasks: state.tasks.map((t) => (t.id === id ? { ...t, model } : t)) });
  if (state.activeId === id) await rebind(id);
  scheduleSave();
  toast("已切换模型：" + modelLabel(state.settings, model), "ok");
}

/** 切换某个本机任务使用的人设（输入区的人设下拉与命令面板都走这里）。 */
export async function setTaskPersona(id: string, persona: string): Promise<void> {
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  const task = state.tasks.find((t) => t.id === id);
  if (!task) return;
  commit({ tasks: state.tasks.map((t) => (t.id === id ? { ...t, persona } : t)) });
  if (state.activeId === id) await rebind(id);
  scheduleSave();
  toast("已切换人设：" + personaLabel(state.settings, persona), "ok");
}

/** 切换某个任务的置顶状态；置顶的任务排在列表最前。 */
export function togglePin(id: string): void {
  const task = state.tasks.find((t) => t.id === id);
  if (!task) return;
  commit({ tasks: replaceTask(id, { pinned: !task.pinned }) });
  scheduleSave();
  toast(task.pinned ? "已取消置顶" : "已置顶", "ok");
}

/* ------------------------------------------------------------ 文件夹选择 */

export function openFolderPick(pick: FolderPick): void {
  commit({ folderPick: pick });
}

export function closeFolderPick(): void {
  if (state.folderPick) commit({ folderPick: null });
}

/** 选定文件夹：为已有任务改工作区，或按这个文件夹新建一个本机任务。 */
export async function confirmFolder(path: string): Promise<void> {
  const pick = state.folderPick;
  const dir = path.trim();
  if (!dir) return;
  commit({ folderPick: null });
  if (!pick) return;
  if (pick.forTask) await setTaskWorkspace(pick.forTask, dir);
  else await createTask(false, "local", dir);
}

export async function activate(id: string, force = false): Promise<void> {
  if (state.busy && !force) {
    toast("任务正在进行中，请稍候");
    return;
  }
  if (!state.tasks.some((t) => t.id === id)) return;
  commit({ activeId: id, filter: "all", preview: null });
  await rebind(id);
  scheduleSave();
}

export async function resetCurrent(): Promise<void> {
  const task = activeTask();
  if (!task) return;
  if (state.busy) {
    toast("任务正在进行中，请稍候");
    return;
  }
  commit({
    tasks: replaceTask(task.id, {
      msgs: [],
      files: [],
      title: "新任务",
      queue: [],
      updatedAt: Date.now(),
    }),
  });
  try {
    await invoke("reset_task");
  } catch {
    /* 忽略：agent 未初始化时 reset 无意义 */
  }
  save();
  toast("已清空当前任务的对话上下文", "ok");
}

export async function deleteTask(id: string): Promise<void> {
  // Deleting the task that owns the in-flight turn would orphan the run: its
  // stream writes would no-op and the trailing `finally` would clear `busy` on
  // whatever task happens to be open.
  if (runTaskId === id) {
    toast("该任务正在执行，请先停止");
    return;
  }
  const index = state.tasks.findIndex((t) => t.id === id);
  if (index < 0) return;
  const tasks = state.tasks.filter((t) => t.id !== id);
  commit({ tasks });
  try {
    await invoke("delete_task", { id });
  } catch {
    /* 忽略：文件可能已不存在 */
  }
  if (state.activeId === id) {
    const next = tasks[Math.max(0, index - 1)];
    if (next) await activate(next.id, true);
    else await createTask(true);
  } else {
    save();
  }
  toast("任务已删除", "ok");
}

export function renameTask(id: string, title: string): void {
  const v = title.trim();
  if (!v) return;
  commit({ tasks: replaceTask(id, { title: v }) });
  scheduleSave();
}

export async function setMode(mode: string): Promise<void> {
  // 持久化走常规设置通道（save_settings_cmd 是唯一写盘者，按到达顺序落盘）；
  // 后端的 set_mode 只更新活动 agent 与内存设置，不再自己写整份快照，
  // 否则可能用一份缺少并发改动的旧快照覆盖 API Key。
  if (state.settings) autosaveSettings({ ...state.settings, mode });
  else commit({ mode });
  try {
    await invoke("set_mode", { mode });
  } catch (e) {
    toast("切换模式失败：" + errText(e), "err");
  }
}

/* ----------------------------------------------------------------- 设置 */

let settingsTimer: number | undefined;

/** 立即把当前设置写回后端。自动保存与手动保存最终都走这里，返回值表示成败。 */
export async function flushSettings(): Promise<boolean> {
  const settings = state.settings;
  if (!settings) return false;
  if (settingsTimer !== undefined) {
    window.clearTimeout(settingsTimer);
    settingsTimer = undefined;
  }
  commit({ settingsSync: "saving", settingsSyncError: "" });
  try {
    await invoke("save_settings_cmd", { settings });
    commit({ settingsSync: "saved" });
    return true;
  } catch (e) {
    const msg = errText(e);
    commit({ settingsSync: "error", settingsSyncError: msg });
    toast("设置保存失败：" + msg, "err");
    return false;
  }
}

/**
 * 改动即保存：新值先落到内存状态（界面立刻生效），再去抖写盘。
 * 用户不必再找保存按钮，输入一半关掉设置页也不会丢。
 */
export function autosaveSettings(next: Settings): void {
  commit({ settings: next, mode: next.mode, settingsSync: "pending", settingsSyncError: "" });
  if (settingsTimer !== undefined) window.clearTimeout(settingsTimer);
  settingsTimer = window.setTimeout(() => {
    settingsTimer = undefined;
    void flushSettings();
  }, 700);
}

/**
 * 关闭设置页时的收尾：确保改动已落盘，并按新配置重绑当前任务的 agent
 * （否则改了密钥还要等下次发消息才生效）。
 */
export function settleSettings(): void {
  void (async () => {
    if (!(await flushSettings())) return;
    const task = activeTask();
    if (task) await rebind(task.id);
  })();
}

/* ------------------------------------------------------------ 结果区 */

const MODIFY_TOOLS = ["write_file", "edit_file", "apply_patch"];

function recordTouch(name: string, summary: string): void {
  const path = (summary || "").trim();
  if (!path) return;
  const kind: Kind | null = MODIFY_TOOLS.includes(name)
    ? "w"
    : name === "read_file"
      ? "r"
      : null;
  if (!kind) return;
  const task = runTask();
  if (!task) return;
  const files = task.files.slice();
  const i = files.findIndex((f) => f.path === path);
  if (i >= 0) files[i] = { ...files[i], kind: kind === "w" ? "w" : files[i].kind, ts: Date.now() };
  else files.push({ path, kind, ts: Date.now() });
  commit({ tasks: replaceTask(task.id, { files }) });
  scheduleSave();
}

/** 每次打开预览递增；只有最后一次请求的响应能写回，避免慢响应覆盖新选择。 */
let previewSeq = 0;

export async function openPreview(path: string): Promise<void> {
  const seq = ++previewSeq;
  commit({ preview: { path, text: null, loading: true, error: null } });
  try {
    const text = await invoke<string>("read_file_preview", { path });
    if (seq !== previewSeq) return;
    commit({ preview: { path, text: text || "", loading: false, error: null } });
  } catch (e) {
    if (seq !== previewSeq) return;
    commit({ preview: { path, text: null, loading: false, error: errText(e) } });
  }
}

export function closePreview(): void {
  // 关闭后到达的旧响应不能再把预览窗重新打开。
  previewSeq++;
  if (state.preview) commit({ preview: null });
}

/* --------------------------------------------------------------- 流式输出 */

let streamIndex = -1;
let pendingDelta = "";
let produced = false;

/**
 * 流式实时文本：走独立通道，不进 React 全局状态。
 *
 * 每 40ms 一次的全局 commit 会让整棵应用树（侧栏、结果区、顶栏、输入区、虚拟
 * 列表的高度回测效应）整体重渲染一遍，还会连带触发强制重排；输出长回答时主线程
 * 就被这些和当前气泡毫无关系的工作占满，界面卡住、鼠标都没反应。改成独立通道后
 * 每帧只有真正显示这条气泡的 Markdown 订阅者拿到新文本，其它组件完全不动；文本
 * 在这一阶段结束时才一次性落进状态。
 */
let streamText = "";
const streamListeners = new Set<() => void>();

export function subscribeStreamText(l: () => void): () => void {
  streamListeners.add(l);
  return () => {
    streamListeners.delete(l);
  };
}

export function getStreamText(): string {
  return streamText;
}

const noopSubscribe = () => () => {};

/** 订阅实时文本；`active` 为 false 时不订阅，避免无关气泡被每帧唤醒。 */
export function useStreamText(active: boolean): string {
  return useSyncExternalStore(
    active ? subscribeStreamText : noopSubscribe,
    getStreamText,
    getStreamText,
  );
}

function setStreamText(t: string): void {
  if (streamText === t) return;
  streamText = t;
  streamListeners.forEach((l) => l());
}

/**
 * 合并流式增量的间隔（约 25fps）。逐帧渲染在这里没有收益：每提交一次都要重新
 * 解析 Markdown、重建消息 DOM、重排整段文字，而这些开销与输出速度无关。
 */
const STREAM_FLUSH_MS = 40;
let flushTimer: number | undefined;

function flushDelta(): void {
  if (flushTimer !== undefined) {
    window.clearTimeout(flushTimer);
    flushTimer = undefined;
  }
  const text = pendingDelta;
  pendingDelta = "";
  if (!text || streamIndex < 0) return;
  setStreamText(streamText + text);
}

function forceFlush(): void {
  flushDelta();
}

function onDelta(text: string): void {
  produced = true;
  if (streamIndex < 0) {
    streamIndex = pushMsg({ k: "ai", t: "", ts: Date.now(), streaming: true });
    setStreamText("");
  }
  pendingDelta += text;
  if (flushTimer === undefined) flushTimer = window.setTimeout(flushDelta, STREAM_FLUSH_MS);
}

/**
 * 一轮里的每个阶段（模型输出 text → 调用工具 → 再输出 text）各自成为一个气泡，
 * 这样工具行会出现在两段文字中间，顺序与真实过程一致。
 */
function closeStream(): void {
  forceFlush();
  const i = streamIndex;
  if (i >= 0) {
    // 实时文本一直在独立通道里，直到这一刻才写回状态（消息的终态文本）。
    if (streamText.trim()) setMsg(i, { t: streamText, streaming: false });
    else dropMsg(i); // 模型先调工具再说话时，空气泡没有意义
  }
  streamIndex = -1;
  // 清掉通道：下一条气泡开始时不能先闪出上一段的旧文本。
  setStreamText("");
  // 这一轮（或这一阶段）结束了，把流式期间攒下的变更落盘一次。
  flushSave();
}

function onTool(p: { name?: string; summary?: string; output?: string }): void {
  produced = true;
  closeStream();
  pushMsg({
    k: "tool",
    name: p.name || "tool",
    summary: p.summary || "",
    out: p.output || "",
    ts: Date.now(),
  });
  recordTouch(p.name || "", p.summary || "");
}

function onNote(text: string): void {
  produced = true;
  closeStream();
  pushMsg({ k: "note", t: text, ts: Date.now() });
}

/**
 * 后端每次模型请求完成后上报「这一次新增的用量」，本机与云端两条路都如此（云端
 * 由 daemon 算差值转发）。这里把它们累加起来，得到该任务的累计用量。
 */
function onUsage(d: Usage): void {
  const task = runTask();
  if (!task) return;
  const prev = task.usage;
  const usage: Usage = {
    requests: (prev?.requests ?? 0) + d.requests,
    prompt_tokens: (prev?.prompt_tokens ?? 0) + d.prompt_tokens,
    completion_tokens: (prev?.completion_tokens ?? 0) + d.completion_tokens,
  };
  commit({ tasks: replaceTask(task.id, { usage }) });
  scheduleSave();
}

/**
 * 开发环境专用：造一个本机任务并返回「模拟流式输出」的句柄。
 *
 * 桌面端靠 Tauri 事件通道驱动流式，浏览器里没有这条通道，所以压测（量测边输出
 * 边渲染的真实帧耗时）需要一个入口。它走的是和真机完全一样的路径：`delta` 直接
 * 调用真实的 `onDelta`，因此 40ms 合并、增量渲染、滚动跟随这些行为都被如实复现。
 * 只在 `import.meta.env.DEV` 分支里被引用，生产构建里会被静态剔除。
 */
export function devBench(history = 0): {
  delta: (t: string) => void;
  end: () => void;
} {
  const now = Date.now();
  const msgs: Msg[] = [];
  for (let i = 0; i < history; i++) {
    msgs.push({ k: "user", t: `第 ${i + 1} 问：帮我看看这个模块的实现。`, ts: now + i * 1000 });
    msgs.push({
      k: "ai",
      t: `第 ${i + 1} 答：\n\n这个模块分三层。\n\n\`\`\`rust\nfn demo_${i}() {}\n\`\`\`\n\n下面是比较长的一段说明文字，用来撑起消息高度。`,
      ts: now + i * 1000 + 500,
    });
  }
  const id = newId();
  const task: Task = {
    id,
    title: "流式压测",
    createdAt: now,
    updatedAt: now,
    msgs,
    files: [],
    location: "local",
    workspace: "",
    model: "",
    persona: "",
  };
  commit({ tasks: [task, ...state.tasks], activeId: id });
  streamIndex = -1;
  return {
    delta: (t: string) => onDelta(t),
    end: () => closeStream(),
  };
}

/** 订阅后端事件；返回取消订阅函数（组件卸载时调用）。 */
export function wireEvents(): () => void {
  const unlistens: Array<() => void> = [];
  let disposed = false;
  const keep = (u: () => void) => {
    if (disposed) u();
    else unlistens.push(u);
  };
  void listen<string>(EVENT_DELTA, onDelta).then(keep);
  void listen<{ name?: string; summary?: string; output?: string }>(EVENT_TOOL, onTool).then(keep);
  void listen<string>(EVENT_NOTE, onNote).then(keep);
  void listen<Usage>(EVENT_USAGE, onUsage).then(keep);
  return () => {
    disposed = true;
    unlistens.forEach((u) => u());
    unlistens.length = 0;
  };
}

/* ------------------------------------------------------------------ 发送 */

/** 把一条消息的文本载入输入框，改完即可重新发送。 */
export function editInComposer(text: string): void {
  commit({ prompt: text, focusTick: state.focusTick + 1 });
}

/** 把一段提示词片段插到输入框末尾（前面已有内容时补一个换行），并聚焦输入框。 */
export function insertSnippet(text: string): void {
  const frag = (text || "").trim();
  if (!frag) return;
  const cur = state.prompt.replace(/\s+$/, "");
  commit({ prompt: cur ? cur + "\n" + frag : frag, focusTick: state.focusTick + 1 });
}

/**
 * 发送一条消息。`text` / `images` 不传时取输入框里的内容与已附加的图片（重试时由
 * 调用方显式传入历史消息的原文与图片）。
 */
export async function send(text?: string, images?: string[]): Promise<void> {
  // 从输入框发出的消息（没有显式传入原文），运行中要进队列，而不是被丢弃。
  const fromComposer = text === undefined;
  const raw = (text !== undefined ? text : state.prompt).trim();
  const attached = images ?? state.attachments.map((a) => a.url);
  // 只有图片、没有文字也是合法的提问（例如「看看这张截图」的直接拖入）。
  if (!raw && attached.length === 0) return;
  if (!activeTask()) await createTask(true);
  const task = activeTask();
  if (!task) return;
  if (state.busy) {
    if (!fromComposer) {
      // 重试旧消息需要显式排队位置，运行中直接重试会打乱当前轮次。
      toast("上一次回答还在进行中，完成或停止后再重试");
      return;
    }
    const n = (task.queue?.length || 0) + 1;
    if (raw) pushHistory(raw);
    enqueue(task, {
      text: raw,
      imgs: attached.length ? attached : undefined,
      refs: state.refs.slice(),
    });
    // 输入框内容已经交给队列，和正常发送一样清空，避免下一轮重复带上。
    commit({ prompt: "", refs: [], attachments: [] });
    toast(
      state.stopping
        ? `已加入队列（${n} 条），停止后可点发送继续`
        : `已加入队列（${n} 条），本轮结束后自动发送`,
      "ok",
    );
    return;
  }
  await runTurn(task, raw, attached, state.refs.slice(), {
    remember: fromComposer,
    clearComposer: true,
    useRefs: fromComposer && !!raw,
  });
}

/** 发送/排队共用的执行入口：绑定任务、落用户消息、跑完整一轮。 */
async function runTurn(
  task: Task,
  raw: string,
  attached: string[],
  refs: string[],
  opts: { remember: boolean; clearComposer: boolean; useRefs: boolean },
): Promise<void> {
  // 这一轮绑定到当前任务：运行中切到别的任务时，增量/工具行/用量都落回它自己。
  runTaskId = task.id;

  const payload =
    opts.useRefs && refs.length ? "（请参考工作区文件：" + refs.join("、") + "）\n" + raw : raw;
  // 只有真正从输入框发出的提问进历史；重试旧消息、队列续发不重复记。
  if (opts.remember && raw) pushHistory(raw);

  pushMsg({ k: "user", t: payload, imgs: attached.length ? attached : undefined, ts: Date.now() });
  if (task.title === "新任务") {
    commit({ tasks: replaceTask(task.id, { title: titleFrom(raw || "图片") }) });
  }
  // 已经随这条消息发出的引用与图片不再留在输入框里（重试时用消息自身的图片）。
  commit({
    prompt: opts.clearComposer ? "" : state.prompt,
    busy: true,
    stopping: false,
    refs: opts.clearComposer ? [] : state.refs,
    attachments: opts.clearComposer ? [] : state.attachments,
  });

  produced = false;
  pendingDelta = "";
  // 执行位置跟着任务走，而不是全局设置。
  const cloud = taskIsCloud(task);
  // 本机与云端都是真流式：远端 daemon 以 NDJSON 逐行下发，GUI 把增量转成同一套
  // 事件通道，因此这里共用同一个流式气泡，不再区分两条收尾路径。
  streamIndex = pushMsg({ k: "ai", t: "", ts: Date.now(), streaming: true });

  try {
    const response = await invoke<string>("run_task", {
      prompt: payload,
      location: cloud ? "cloud" : "local",
      images: attached,
    });
    // 增量事件可能晚于 invoke 返回一瞬到达：先强制刷入，否则末尾的增量会被
    // 以 streaming:true 追加，气泡永远停在流式状态。
    forceFlush();
    // 实时文本不在消息里（流式期间消息的 t 恒为空串，避免每帧全局重渲染），
    // 终态文本要从独立通道取；通道为空时退回 invoke 的返回值。
    const streamed = streamText;
    const i = streamIndex;
    streamIndex = -1;
    if (i >= 0) {
      const t = streamed.trim() ? streamed : response || "";
      if (t.trim()) setMsg(i, { t, streaming: false });
      else dropMsg(i);
    } else if (response?.trim()) {
      pushMsg({ k: "ai", t: response, ts: Date.now() });
    }
    // 主动停止时没有输出是正常的，不要补一条「空响应」。
    if (!state.stopping && !produced && !(response && response.trim())) {
      pushMsg({ k: "ai", t: "（空响应）", ts: Date.now() });
    }
  } catch (e) {
    const msg = errText(e);
    // 用户主动停止时连接被后端主动断开（或读超时），不是故障：保留已有输出，
    // 不要补红色错误气泡；收尾的 finally 会给出「任务已停止」回执。
    const stopped = state.stopping;
    forceFlush();
    const streamed = streamText;
    const i = streamIndex;
    streamIndex = -1;
    if (stopped) {
      if (i >= 0) {
        if (streamed.trim()) setMsg(i, { t: streamed, streaming: false });
        else dropMsg(i);
      }
    } else if (i >= 0 && streamed.trim()) {
      // 已经输出了一部分：保留内容，把中断原因作为备注追加在后面。
      setMsg(i, { t: streamed, streaming: false });
      pushMsg({ k: "note", t: "运行中断：" + msg, ts: Date.now() });
    } else if (i >= 0) {
      setMsg(i, { t: "错误：" + msg, err: true, streaming: false });
    } else {
      pushMsg({ k: "ai", t: "错误：" + msg, err: true, ts: Date.now() });
    }
  } finally {
    // 兜底定稿：竞态下晚到的增量可能又开了一个流式气泡，这里统一收掉。
    closeStream();
    streamIndex = -1;
    pendingDelta = "";
    produced = false;
    runTaskId = null;
    // 停止请求只在步骤边界生效：本轮真正结束的这一刻才复位，并给一次明确回执，
    // 否则点了停止之后界面没有任何变化，用户无法判断请求是否已经生效。
    const stopped = state.stopping;
    commit({ busy: false, stopping: false });
    save();
    if (stopped) toast("任务已停止", "ok");
    // 正常结束后按顺序续发队列；主动停止时不自动续跑，把队列留给用户决定。
    else window.setTimeout(() => drainQueue(task.id), 0);
  }
}

/** 把一条消息追加到任务的待发送队列。 */
function enqueue(task: Task, item: QueueItem): void {
  setQueue(task.id, [...(task.queue || []), item]);
}

function setQueue(taskId: string, queue: QueueItem[]): void {
  commit({ tasks: replaceTask(taskId, { queue }) });
  scheduleSave();
}

/** 取当前任务队列里的第一条立即发送（停止后由用户手动触发）。 */
export function sendQueued(): void {
  const task = activeTask();
  if (task) drainQueue(task.id);
}

/** 移出队列中的某一条（按当前任务的下标）。 */
export function removeQueued(index: number): void {
  const task = activeTask();
  const queue = task?.queue;
  if (!task || !queue || index < 0 || index >= queue.length) return;
  setQueue(
    task.id,
    queue.filter((_, i) => i !== index),
  );
}

/**
 * 续发队首。只在该任务仍是当前打开的任务时自动执行：用户运行中切到别的任务时，
 * 不能把回答悄悄写进另一个任务的界面。
 */
function drainQueue(taskId: string): void {
  if (state.busy || state.activeId !== taskId) return;
  const task = state.tasks.find((t) => t.id === taskId);
  const queue = task?.queue || [];
  const item = queue[0];
  if (!task || !item) return;
  setQueue(taskId, queue.slice(1));
  void runTurn(task, item.text, item.imgs || [], item.refs || [], {
    remember: false,
    clearComposer: false,
    useRefs: item.refs.length > 0,
  });
}

/** 长回答折叠/展开。 */
export function toggleFold(index: number): void {
  const task = activeTask();
  const m = task?.msgs[index];
  if (!task || !m || m.k !== "ai" || m.streaming) return;
  const msgs = task.msgs.slice();
  msgs[index] = { ...m, fold: !m.fold };
  commit({ tasks: replaceTask(task.id, { msgs }) });
  scheduleSave();
}

/** 正在运行的任务 id（用于判断队列是否会自动续发）。 */
export const runningTaskId = (): string | null => runTaskId;

/** 重试某条回答：复用其前面最近的一条用户消息（含当时的图片）。 */
export function retryFrom(index: number): void {
  const task = activeTask();
  if (!task) return;
  for (let i = index - 1; i >= 0; i--) {
    if (task.msgs[i].k === "user") {
      // 显式传数组（可能是空数组）：这样重试只重发那条消息自己的图片，
      // 不会把输入框里尚未发出的附件顺手带上，也不会把它们清空。
      void send(task.msgs[i].t || "", task.msgs[i].imgs || []);
      return;
    }
  }
}

/* -------------------------------------------------------------- 停止 / 导出 */

/**
 * 请求停止正在执行的任务。后端只在「下一个步骤的开头」检查停止标志，所以当前
 * 已经发出的模型请求或工具调用仍会跑完，这一点如实告诉用户。
 */
export async function stop(): Promise<void> {
  if (!state.busy || state.stopping) return;
  try {
    await invoke("stop_task");
    // 停止标志在下一次进入步骤边界时才会被读到，这段等待可能持续一个完整的
    // 模型请求或工具调用。把「停止中」标出来，按钮才不会看起来点了没反应。
    if (state.busy) commit({ stopping: true });
    toast("已请求停止，将在当前步骤结束后生效");
  } catch (e) {
    toast("停止失败：" + errText(e), "err");
  }
}

/** 把当前任务的对话渲染成 Markdown，便于归档或分享。 */
function taskToMarkdown(task: Task): string {
  const head = [
    `# ${task.title}`,
    "",
    `- 导出时间：${new Date().toLocaleString()}`,
    `- 执行位置：${task.location === "cloud" ? "云端" : "本机"}`,
  ];
  if (task.workspace) head.push(`- 工作区：${task.workspace}`);
  head.push("");

  const body: string[] = [];
  for (const m of task.msgs) {
    if (m.k === "user") {
      body.push("## 用户", "", m.t || "", "");
    } else if (m.k === "ai") {
      body.push("## NanoHermes", "", m.t || "", "");
    } else if (m.k === "tool") {
      body.push(`> 工具调用 \`${m.name || "tool"}\` ${m.summary || ""}`, "");
      const out = (m.out || "").trim();
      if (out) body.push("```", out, "```", "");
    } else if (m.k === "note") {
      body.push(`> ${m.t || ""}`, "");
    }
  }
  return [...head, ...body].join("\n") + "\n";
}

/** 导出当前任务的对话为 Markdown 文件，成功后提示落盘的绝对路径。 */
export async function exportCurrent(): Promise<void> {
  const task = activeTask();
  if (!task) return;
  if (!task.msgs.length) {
    toast("当前任务还没有对话内容");
    return;
  }
  try {
    const path = await invoke<string>("export_task", {
      name: task.title,
      text: taskToMarkdown(task),
    });
    toast("已导出到 " + path, "ok");
  } catch (e) {
    toast("导出失败：" + errText(e), "err");
  }
}

/* ------------------------------------------------------------------ 启动 */

export async function boot(): Promise<void> {
  load();
  try {
    const s = await invoke<Settings>("get_settings");
    commit({ settings: s, mode: s.mode || "default" });
  } catch {
    commit({ settings: null });
  }
  if (!state.tasks.length) {
    await createTask(true);
  } else {
    const target =
      state.tasks.find((t) => t.id === state.activeId) ?? state.tasks[state.tasks.length - 1];
    await activate(target.id, true);
  }
}

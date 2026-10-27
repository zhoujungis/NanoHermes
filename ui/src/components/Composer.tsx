import { useEffect, useRef, useState } from "react";
import {
  activeTask,
  addAttachment,
  inputHistory,
  insertAt,
  modelLabel,
  personaIsCustom,
  personaLabel,
  removeAttachment,
  removeQueued,
  removeRef,
  resolveModel,
  resolvePersona,
  runningTaskId,
  send,
  sendQueued,
  setMode,
  setPrompt,
  stop,
  taskIsCloud,
  useAppState,
} from "../lib/store";
import { formatBytes, imageFilesFrom } from "../lib/image";
import { Icon } from "./Icon";
import { ModelMenu } from "./ModelMenu";
import { PersonaMenu } from "./PersonaMenu";
import { SnippetMenu } from "./SnippetMenu";

/** 模式切换：值、短标签、说明（与后端 PermissionMode 一一对应）。 */
const MODES: Array<[string, string, string]> = [
  ["default", "Agent", "全部工具可用，文件修改与命令自动执行"],
  ["plan", "Plan", "只读分析：不写文件、不执行命令"],
  ["goal", "Goal", "目标模式：无人值守，所有工具自动执行直到完成"],
  ["debug", "Debug", "诊断模式：可读文件、可执行命令，但不修改文件"],
];

/** 输入区：引用文件、模式切换、发送。 */
export function Composer({
  onOpenPicker,
  onOpenSettings,
}: {
  /** 打开文件选择器；`onPick` 不传时选中即加入引用，传了则把路径插回输入框（@ 触发）。 */
  onOpenPicker: (rect: DOMRect, onPick?: (path: string) => void) => void;
  onOpenSettings: () => void;
}) {
  const st = useAppState();
  const task = activeTask();
  // 执行位置属于当前任务：云端任务不显示本机的模式切换，因为远端模式由
  // daemon 的启动参数决定，本机改了也没用。
  const cloud = taskIsCloud(task);
  // 云端执行时模型密钥在服务端，本机只需一个可用的云服务地址。
  const model = modelLabel(st.settings, task?.model);
  // 人设同样只对本机任务生效，云端行为由 daemon 决定。
  const persona = personaLabel(st.settings, task?.persona);
  const personaOn = personaIsCustom(resolvePersona(st.settings, task?.persona));
  const ready = cloud
    ? !!(st.settings?.cloud_url || "").trim()
    : !!(resolveModel(st.settings, task?.model)?.api_key || "").trim();
  // 当前任务里排队等待发送的消息；只有本任务正在跑时才会自动续发。
  const queue = task?.queue ?? [];
  const queueAuto = st.busy && runningTaskId() === task?.id;
  const queueHint = queueAuto
    ? "本轮结束后自动发送"
    : st.busy
      ? "其他任务运行中，结束后点发送"
      : "点击发送继续";
  const ta = useRef<HTMLTextAreaElement>(null);
  const plus = useRef<HTMLButtonElement>(null);
  const modelBtn = useRef<HTMLButtonElement>(null);
  const personaBtn = useRef<HTMLButtonElement>(null);
  const snippetBtn = useRef<HTMLButtonElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  /** 拖拽图片到输入区时的落点高亮。 */
  const [dropping, setDropping] = useState(false);
  /** 当前打开的底栏下拉（模型 / 人设 / 片段）；null 表示都没开。 */
  const [menu, setMenu] = useState<{ kind: "model" | "persona" | "snippet"; rect: DOMRect } | null>(
    null,
  );
  /** ↑↓ 输入历史的浏览位置；null 表示没在翻历史。 */
  const histIdx = useRef<number | null>(null);
  /** 开始翻历史前的草稿，翻回最新一条时恢复。 */
  const draft = useRef("");

  /** 翻到输入历史的某一条；idx 为 null 表示回到草稿。 */
  const recall = (idx: number | null) => {
    const hist = inputHistory();
    if (idx === null) {
      histIdx.current = null;
      setPrompt(draft.current);
      return;
    }
    const i = Math.max(0, Math.min(idx, hist.length - 1));
    histIdx.current = i;
    setPrompt(hist[i]);
    // 翻到的条目替换整段输入，光标挪到末尾方便直接续写。
    requestAnimationFrame(() => {
      const el = ta.current;
      if (el) el.setSelectionRange(el.value.length, el.value.length);
    });
  };

  /** 以某个按钮为锚点打开对应的底栏下拉。 */
  const openMenu = (kind: "model" | "persona" | "snippet", el: HTMLElement | null) => {
    setMenu({
      kind,
      rect: el?.getBoundingClientRect() ?? new DOMRect(16, window.innerHeight - 60, 0, 0),
    });
  };

  const attachFiles = (files: File[]) => {
    files.forEach((f) => void addAttachment(f));
  };

  // 输入框随内容增高，最多 200px。
  useEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  }, [st.prompt]);

  // 从消息里点「编辑」时，把焦点交回输入框。
  useEffect(() => {
    if (st.focusTick > 0) ta.current?.focus();
  }, [st.focusTick]);

  // 点过停止之后，停止按钮会随本轮结束一起消失：焦点如果正停在它上面就会直接
  // 掉到 body，键盘用户还得重新 Tab 回来。只在这种「本来就在操作本轮状态」的
  // 情况下收焦点，不打扰正在别处选字或填表单的人。
  const wasStopping = useRef(false);
  useEffect(() => {
    if (st.busy) {
      wasStopping.current = st.stopping;
      return;
    }
    if (!wasStopping.current) return;
    wasStopping.current = false;
    if (!document.activeElement || document.activeElement === document.body) ta.current?.focus();
  }, [st.busy, st.stopping]);

  // 底栏下拉：点空白处或按 Esc 收起（下拉自身与其锚点除外）。
  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (!t?.closest("[data-menu-pop]") && !t?.closest("[data-menu-anchor]")) setMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null);
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  return (
    <div className="composer-wrap">
      {!ready && (
        <div className="notice">
          <Icon name="info" />
          <span>
            {cloud
              ? "还没有填写云服务地址，配置后即可开始在远端执行。"
              : "还没有配置模型服务，填写 API Key 后即可开始。"}
          </span>
          <button onClick={onOpenSettings}>去设置</button>
        </div>
      )}

      {queue.length > 0 && (
        <div className="queuebar" aria-label="待发送队列">
          <div className="qh">
            <Icon name="arrowUp" />
            <span className="qc">队列 {queue.length} 条</span>
            <span className="qs">{queueHint}</span>
            <button
              className="qsend"
              disabled={st.busy}
              title={queueHint}
              onClick={() => sendQueued()}
            >
              发送
            </button>
          </div>
          {queue.map((q, i) => (
            <div className="qi" key={i}>
              {q.imgs?.length ? <span className="qtag">图 {q.imgs.length}</span> : null}
              <span className="qtext" title={q.text}>
                {q.text || "（仅图片）"}
              </span>
              <button title="移出队列" onClick={() => removeQueued(i)}>
                <Icon name="x" />
              </button>
            </div>
          ))}
        </div>
      )}

      <div
        className={"composer" + (dropping ? " dropping" : "")}
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes("Files")) {
            e.preventDefault();
            setDropping(true);
          }
        }}
        onDragLeave={(e) => {
          // 只有真正离开整个输入区才取消高亮（子元素之间移动也会触发 leave）。
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setDropping(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDropping(false);
          attachFiles(imageFilesFrom(e.dataTransfer.files));
        }}
      >
        <div className="box">
          {st.attachments.length > 0 && (
            <div className="imgs">
              {st.attachments.map((a) => (
                <div className="img" key={a.url}>
                  <img src={a.url} alt={a.name} />
                  <div className="icap" title={`${a.name} · ${a.w}×${a.h} · ${formatBytes(a.bytes)}`}>
                    {a.w}×{a.h} · {formatBytes(a.bytes)}
                  </div>
                  <button title="移除这张图片" onClick={() => removeAttachment(a.url)}>
                    <Icon name="x" />
                  </button>
                </div>
              ))}
            </div>
          )}

          {st.refs.length > 0 && (
            <div className="refs">
              {st.refs.map((p) => (
                <div className="ref" key={p}>
                  <span title={p}>{p}</span>
                  <button title="移除引用" onClick={() => removeRef(p)}>
                    <Icon name="x" />
                  </button>
                </div>
              ))}
            </div>
          )}

          <textarea
            id="prompt"
            ref={ta}
            rows={1}
            spellCheck={false}
            aria-label="输入任务"
            value={st.prompt}
            placeholder="描述一个任务，例如：为 src/tools/search.rs 补充单元测试"
            onChange={(e) => {
              // 手动编辑即离开历史浏览状态，草稿以最新输入为准。
              histIdx.current = null;
              setPrompt(e.target.value);
            }}
            onPaste={(e) => {
              // 截图直接粘贴：把剪贴板里的图片收进附件，不让它变成文件名文本。
              const files = imageFilesFrom(e.clipboardData?.items ?? null);
              if (files.length) {
                e.preventDefault();
                attachFiles(files);
              }
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                histIdx.current = null;
                void send();
                return;
              }
              const el = e.currentTarget;
              const caret = el.selectionStart ?? el.value.length;
              const hist = inputHistory();
              // ↑↓ 翻输入历史：只在光标位于首行 / 末行时接管，不影响多行内的正常移动。
              if (e.key === "ArrowUp" && hist.length > 0 && !el.value.slice(0, caret).includes("\n")) {
                e.preventDefault();
                if (histIdx.current === null) {
                  draft.current = el.value;
                  recall(hist.length - 1);
                } else {
                  recall(histIdx.current - 1);
                }
                return;
              }
              if (e.key === "ArrowDown" && histIdx.current !== null) {
                e.preventDefault();
                const next = histIdx.current + 1;
                if (next >= hist.length) recall(null);
                else recall(next);
                return;
              }
              // 输入 @（前面是空白或行首）唤起文件选择器，选中后把路径插到 @ 后面。
              if (e.key === "@" && !e.nativeEvent.isComposing) {
                const before = el.value.slice(0, caret);
                if (!before || /\s$/.test(before)) {
                  e.preventDefault();
                  // 把 @ 和选中的路径一起插回光标处。
                  onOpenPicker(el.getBoundingClientRect(), (path) =>
                    insertAt(caret, "@" + path + " "),
                  );
                }
              }
            }}
          />

          <input
            ref={fileInput}
            type="file"
            accept="image/*"
            multiple
            hidden
            onChange={(e) => {
              attachFiles(imageFilesFrom(e.target.files));
              // 允许再次选择同一个文件。
              e.target.value = "";
            }}
          />

          <div className="cbar">
            <button
              ref={plus}
              className="cbtn"
              title="引用工作区文件"
              data-picker-anchor
              onClick={() =>
                onOpenPicker(
                  plus.current?.getBoundingClientRect() ??
                    new DOMRect(16, window.innerHeight - 60, 0, 0),
                )
              }
            >
              <Icon name="plus" />
            </button>
            <button
              className="cbtn"
              title="附加图片（也可以直接粘贴或拖入图片）"
              onClick={() => fileInput.current?.click()}
            >
              <Icon name="image" />
            </button>
            <button
              ref={snippetBtn}
              className="cbtn"
              data-menu-anchor
              title="插入提示词片段"
              onClick={() => openMenu("snippet", snippetBtn.current)}
            >
              <Icon name="bolt" />
            </button>

            {!cloud && (
              <>
                <button
                  ref={modelBtn}
                  className="modelpill"
                  data-menu-anchor
                  title={`模型：${model}\n点击切换本机任务使用的模型`}
                  onClick={() => openMenu("model", modelBtn.current)}
                >
                  <Icon name="spark" />
                  <span className="mname">{model}</span>
                  <Icon name="chev" className="mchev" />
                </button>

                <button
                  ref={personaBtn}
                  className={"modelpill persona" + (personaOn ? " on" : "")}
                  data-menu-anchor
                  title={`人设：${persona}\n点击切换本机任务使用的人设`}
                  onClick={() => openMenu("persona", personaBtn.current)}
                >
                  <Icon name="user" />
                  <span className="mname">{persona}</span>
                  <Icon name="chev" className="mchev" />
                </button>
              </>
            )}

            {cloud ? (
              <div
                className="perm"
                title="云端执行：模型服务与权限模式由远端 daemon 的启动参数决定，本机无法切换"
              >
                <Icon name="cloud" />
                <span>云端服务</span>
              </div>
            ) : (
              <div className="seg">
                {MODES.map(([value, label, desc]) => (
                  <button
                    key={value}
                    title={desc}
                    className={st.mode === value ? "on" : ""}
                    onClick={() => void setMode(value)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}

            <div className="sp" />
            <span className="hint">
              {st.busy ? "Enter 加入队列" : "Enter 发送"} · Shift+Enter 换行
            </span>
            {/* 运行中，发送键原地变成停止键（同一个 30px 位置）。两个按钮同时出现
                会把底栏撑出输入框，窄面板下尤其明显。 */}
            {st.busy ? (
              <button
                className={"send stop" + (st.stopping ? " waiting" : "")}
                title={
                  st.stopping
                    ? "已请求停止，正在等待当前步骤结束"
                    : "停止任务（当前步骤结束后生效）"
                }
                disabled={st.stopping}
                onClick={() => void stop()}
              >
                {st.stopping ? <div className="spin" /> : <Icon name="square" />}
              </button>
            ) : (
              <button
                className="send"
                title="发送"
                disabled={!st.prompt.trim() && st.attachments.length === 0}
                onClick={() => void send()}
              >
                <Icon name="arrowUp" />
              </button>
            )}
          </div>
        </div>
      </div>

      {menu?.kind === "model" && task && (
        <ModelMenu id={task.id} rect={menu.rect} onClose={() => setMenu(null)} />
      )}
      {menu?.kind === "persona" && task && (
        <PersonaMenu id={task.id} rect={menu.rect} onClose={() => setMenu(null)} />
      )}
      {menu?.kind === "snippet" && (
        <SnippetMenu rect={menu.rect} onClose={() => setMenu(null)} />
      )}
    </div>
  );
}

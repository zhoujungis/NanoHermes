import { useEffect, useState } from "react";
import { radioGroupKeys } from "../lib/a11y";
import {
  autosaveSettings,
  flushSettings,
  setMode,
  settleSettings,
  useAppState,
  type ModelProfile,
  type Persona,
  type Settings,
  type Snippet,
} from "../lib/store";
import { errText, invoke } from "../lib/tauri";
import { setTheme, useTheme, THEME_META } from "../lib/theme";
import { Icon } from "./Icon";

const MODES: Array<[string, string, string]> = [
  ["default", "Agent", "全部工具可用，文件修改与命令自动执行。"],
  ["accept-edits", "Accept Edits", "自动执行文件修改，命令仍受限制。"],
  ["plan", "Plan", "只读分析：禁止写入文件与执行命令。"],
  ["goal", "Goal", "目标模式：无人值守，所有工具自动执行直到目标完成。"],
  ["debug", "Debug", "诊断模式：可读文件、可执行命令以复现问题，但不修改文件。"],
];

const LOCATIONS: Array<[string, string, string]> = [
  ["local", "本机执行", "Agent 直接在本机进程内运行，使用下面的模型服务与工作区。"],
  [
    "cloud",
    "云端执行",
    "每个任务转发到远程 NanoHermes daemon；模型密钥、工作区与权限模式都在服务端。",
  ],
];

/**
 * 设置页：模型列表、执行位置、工作区、权限模式。
 *
 * 没有「保存」这一步——所有改动都即时写回后端（去抖 700ms），失焦与关闭时再兜底
 * 落盘一次，所以任何时候关掉这一页都不会丢输入；右上角的保存按钮只是给想立刻确认
 * 的人一个明确出口。
 */
export function SettingsPage({ onClose }: { onClose: () => void }) {
  const st = useAppState();
  const [models, setModels] = useState<ModelProfile[]>(st.settings?.models || []);
  const [activeModel, setActiveModel] = useState(st.settings?.active_model || "");
  const [workspace, setWorkspace] = useState(st.settings?.workspace || "");
  const [location, setLocation] = useState(st.settings?.location || "local");
  const [cloudUrl, setCloudUrl] = useState(st.settings?.cloud_url || "");
  const [cloudToken, setCloudToken] = useState(st.settings?.cloud_token || "");
  const [personas, setPersonas] = useState<Persona[]>(st.settings?.personas || []);
  const [activePersona, setActivePersona] = useState(st.settings?.active_persona || "");
  const [snippets, setSnippets] = useState<Snippet[]>(st.settings?.snippets || []);
  const [thinking, setThinking] = useState(!!st.settings?.thinking);
  const [projectDocs, setProjectDocs] = useState(st.settings?.project_docs !== false);
  const [memory, setMemory] = useState(st.settings?.memory !== false);
  const [temperature, setTemperature] = useState(st.settings?.temperature ?? 0);
  const [maxIterations, setMaxIterations] = useState(st.settings?.max_iterations ?? 40);
  const [maxTokens, setMaxTokens] = useState(st.settings?.max_tokens ?? 4096);
  // 三个数字输入框的显示草稿；真正的数值仍保存在上面的数字 state 里。
  const [numberDraft, setNumberDraft] = useState({
    temperature: String(st.settings?.temperature ?? 0),
    max_iterations: String(st.settings?.max_iterations ?? 40),
    max_tokens: String(st.settings?.max_tokens ?? 4096),
  });
  const [probe, setProbe] = useState<{ t: string; ok: boolean } | null>(null);
  const [testing, setTesting] = useState(false);
  // 每个模型档案各测各的连通性（键就是档案 id），互不覆盖。
  const [mprobe, setMprobe] = useState<Record<string, { t: string; ok: boolean }>>({});
  const [reveal, setReveal] = useState<Record<string, boolean>>({});
  const theme = useTheme();

  // 设置可能在这一页打开后才从后端返回：只在本地还没有内容时补一次，不覆盖手改。
  useEffect(() => {
    const s = st.settings;
    if (!s) return;
    setModels((cur) => (cur.length ? cur : s.models || []));
    setActiveModel((cur) => cur || s.active_model || "");
    setPersonas((cur) => (cur.length ? cur : s.personas || []));
    setActivePersona((cur) => cur || s.active_persona || "");
    setSnippets((cur) => (cur.length ? cur : s.snippets || []));
  }, [st.settings]);

  // 关闭这一页时（✕ / Esc / 途中任意方式）确保落盘并让新配置生效。
  useEffect(() => settleSettings, []);

  /** 当前本地状态组成的设置对象；文本字段统一去掉首尾空白。 */
  const draft = (patch: Partial<Settings> = {}): Settings => {
    const cur = st.settings;
    const base: Settings = {
      api_key: cur?.api_key || "",
      base_url: cur?.base_url || "",
      model: cur?.model || "",
      workspace,
      mode: st.mode,
      location,
      cloud_url: cloudUrl,
      cloud_token: cloudToken,
      models,
      active_model: activeModel,
      personas,
      active_persona: activePersona,
      snippets,
      thinking,
      project_docs: projectDocs,
      memory,
      temperature,
      max_iterations: maxIterations,
      max_tokens: maxTokens,
    };
    const s = { ...base, ...patch };
    return {
      ...s,
      workspace: s.workspace.trim(),
      cloud_url: s.cloud_url.trim(),
      cloud_token: s.cloud_token.trim(),
      models: s.models.map((m) => ({
        ...m,
        name: m.name.trim(),
        base_url: m.base_url.trim(),
        api_key: m.api_key.trim(),
        model: m.model.trim(),
      })),
      personas: s.personas.map((p) => ({
        ...p,
        name: p.name.trim(),
        prompt: p.prompt.trim(),
      })),
      snippets: s.snippets.map((x) => ({ ...x, name: x.name.trim(), text: x.text.trim() })),
    };
  };

  const change = (patch: Partial<Settings>): void => autosaveSettings(draft(patch));

  /** 改一个档案的字段：更新本地状态并把整份列表写回后端。 */
  const editModel = (index: number, patch: Partial<ModelProfile>): void => {
    const next = models.map((m, n) => (n === index ? { ...m, ...patch } : m));
    setModels(next);
    change({ models: next });
  };

  const addModel = (): void => {
    const id = "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 5);
    const next = [...models, { id, name: "新模型", base_url: "", api_key: "", model: "" }];
    setModels(next);
    change({ models: next });
  };

  const removeModel = (id: string): void => {
    const next = models.filter((m) => m.id !== id);
    const active = next.some((m) => m.id === activeModel) ? activeModel : next[0]?.id || "";
    setModels(next);
    setActiveModel(active);
    change({ models: next, active_model: active });
  };

  const makeDefault = (id: string): void => {
    setActiveModel(id);
    change({ active_model: id });
  };

  /** 改一个人设：更新本地状态并把整份列表写回后端。 */
  const editPersona = (index: number, patch: Partial<Persona>): void => {
    const next = personas.map((p, n) => (n === index ? { ...p, ...patch } : p));
    setPersonas(next);
    change({ personas: next });
  };

  const addPersona = (): void => {
    const id = "p" + Date.now().toString(36) + Math.random().toString(36).slice(2, 5);
    const next = [
      ...personas,
      { id, name: "新的人设", prompt: "例如：回答一律用中文，先给结论再给理由。" },
    ];
    setPersonas(next);
    change({ personas: next });
  };

  const removePersona = (id: string): void => {
    const next = personas.filter((p) => p.id !== id);
    const active = next.some((p) => p.id === activePersona) ? activePersona : next[0]?.id || "";
    setPersonas(next);
    setActivePersona(active);
    change({ personas: next, active_persona: active });
  };

  const makeDefaultPersona = (id: string): void => {
    setActivePersona(id);
    change({ active_persona: id });
  };

  /** 改一段提示词片段。 */
  const editSnippet = (index: number, patch: Partial<Snippet>): void => {
    const next = snippets.map((x, n) => (n === index ? { ...x, ...patch } : x));
    setSnippets(next);
    change({ snippets: next });
  };

  const addSnippet = (): void => {
    const id = "s" + Date.now().toString(36) + Math.random().toString(36).slice(2, 5);
    const next = [...snippets, { id, name: "新片段", text: "" }];
    setSnippets(next);
    change({ snippets: next });
  };

  const removeSnippet = (id: string): void => {
    const next = snippets.filter((x) => x.id !== id);
    setSnippets(next);
    change({ snippets: next });
  };

  /** 高级参数：切换开关 / 调整数值（空值回落到原来的默认，不写坏配置）。 */
  const setToggle = (key: "thinking" | "project_docs" | "memory", value: boolean): void => {
    if (key === "thinking") {
      setThinking(value);
      change({ thinking: value });
    } else if (key === "project_docs") {
      setProjectDocs(value);
      change({ project_docs: value });
    } else {
      setMemory(value);
      change({ memory: value });
    }
  };

  const setNumber = (key: "temperature" | "max_iterations" | "max_tokens", raw: string): void => {
    // 输入框用字符串草稿，才能先清空再重新输入。直接 `value={number}` 时，
    // 清空会先变成 `Number("") === 0`，被 clamp 成 0 / 1 又弹回来，光标也跟着跳。
    setNumberDraft((d) => ({ ...d, [key]: raw }));
    if (raw.trim() === "") return;
    const v = Number(raw);
    if (!Number.isFinite(v)) return;
    if (key === "temperature") {
      const t = Math.min(2, Math.max(0, v));
      setTemperature(t);
      change({ temperature: t });
    } else if (key === "max_iterations") {
      const n = Math.min(200, Math.max(1, Math.round(v)));
      setMaxIterations(n);
      change({ max_iterations: n });
    } else {
      const n = Math.min(200000, Math.max(1, Math.round(v)));
      setMaxTokens(n);
      change({ max_tokens: n });
    }
  };

  /** 失焦时把空草稿还原成上一次生效的值，避免输入框一直空着。 */
  const settleNumber = (key: "temperature" | "max_iterations" | "max_tokens"): void => {
    if (!numberDraft[key].trim()) {
      const committed =
        key === "temperature" ? temperature : key === "max_iterations" ? maxIterations : maxTokens;
      setNumberDraft((d) => ({ ...d, [key]: String(committed) }));
    }
    void flushSettings();
  };

  const testCloud = async () => {
    setProbe(null);
    setTesting(true);
    try {
      const r = await invoke<string>("cloud_health", {
        url: cloudUrl.trim(),
        token: cloudToken.trim(),
      });
      setProbe({ t: r, ok: true });
    } catch (e) {
      setProbe({ t: errText(e), ok: false });
    } finally {
      setTesting(false);
    }
  };

  const testModel = async (m: ModelProfile) => {
    setMprobe((p) => ({ ...p, [m.id]: { t: "测试中…", ok: true } }));
    try {
      const r = await invoke<string>("model_health", {
        baseUrl: m.base_url.trim(),
        apiKey: m.api_key.trim(),
      });
      setMprobe((p) => ({ ...p, [m.id]: { t: r, ok: true } }));
    } catch (e) {
      setMprobe((p) => ({ ...p, [m.id]: { t: errText(e), ok: false } }));
    }
  };

  const sync = st.settingsSync;
  const status =
    sync === "pending" || sync === "saving"
      ? { t: "保存中…", k: "" }
      : sync === "saved"
        ? { t: "已自动保存", k: "ok" }
        : sync === "error"
          ? { t: "保存失败：" + (st.settingsSyncError || "未知原因"), k: "err" }
          : { t: "改动会自动保存", k: "" };

  return (
    <div className="settings show">
      <div className="st-head">
        <span className="t">设置</span>
        <span className={"sync " + status.k}>{status.t}</span>
        <span className="sp" />
        <button className="ghost" title="立即保存" onClick={() => void flushSettings()}>
          保存
        </button>
        <button className="tbtn" title="关闭（Esc）" onClick={onClose}>
          <Icon name="x" />
        </button>
      </div>

      <div className="st-body">
        <div className="st-col">
          <section className="sect">
            <div className="sh">
              <h3>主题</h3>
              <p>界面配色。选「跟随系统」时，会随 Windows 的浅色/深色设置自动切换。</p>
            </div>
            <div className="sb">
              <div
                className="themepick"
                role="radiogroup"
                aria-label="界面主题"
                onKeyDown={(e) => radioGroupKeys(e)}
              >
                {THEME_META.map(({ id, name, swatch }) => (
                  <button
                    key={id}
                    role="radio"
                    aria-checked={theme === id}
                    tabIndex={theme === id ? 0 : -1}
                    className={theme === id ? "on" : ""}
                    onClick={() => setTheme(id)}
                  >
                    <span className="sw" aria-hidden="true">
                      {swatch.map((c) => (
                        <i key={c} style={{ background: c }} />
                      ))}
                    </span>
                    <span className="t">{name}</span>
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>默认执行位置</h3>
              <p>新建任务时的默认位置；每个任务也可以单独选择在本机或云端执行。</p>
            </div>
            <div className="sb">
              <div
                className="modepick"
                role="radiogroup"
                aria-label="默认执行位置"
                onKeyDown={(e) => radioGroupKeys(e)}
              >
                {LOCATIONS.map(([value, name, desc]) => (
                  <button
                    key={value}
                    role="radio"
                    aria-checked={location === value}
                    tabIndex={location === value ? 0 : -1}
                    className={location === value ? "on" : ""}
                    onClick={() => {
                      setLocation(value);
                      change({ location: value });
                    }}
                  >
                    <span>
                      <span className="k">{name}</span>
                      <span className="v">{desc}</span>
                    </span>
                  </button>
                ))}
              </div>

              <>
                <div className="hintline">下面两项只被云端任务使用，本机任务不需要。</div>
                <div className="field">
                  <label>服务地址</label>
                  <div className="row">
                    <input
                      type="text"
                      value={cloudUrl}
                      spellCheck={false}
                      placeholder="http://203.0.113.7:8787"
                      onChange={(e) => {
                        setCloudUrl(e.target.value);
                        change({ cloud_url: e.target.value });
                      }}
                      onBlur={() => void flushSettings()}
                    />
                    <button
                      className="ghost"
                      disabled={testing || !cloudUrl.trim()}
                      onClick={() => void testCloud()}
                    >
                      {testing ? "测试中…" : "测试连接"}
                    </button>
                  </div>
                  <div
                    className="hintline"
                    style={probe ? { color: probe.ok ? "var(--ok)" : "var(--danger)" } : undefined}
                  >
                    {probe
                      ? probe.t
                      : "daemon 需以 --daemon-bind 0.0.0.0 启动，并放行对应端口；也可用 Caddy 反代成 https。"}
                  </div>
                </div>
                <div className="field">
                  <label>访问 Token</label>
                  <div className="row">
                    <input
                      type={reveal.cloud ? "text" : "password"}
                      value={cloudToken}
                      spellCheck={false}
                      autoComplete="off"
                      placeholder="服务端 NANOHERMES_DAEMON_TOKEN"
                      onChange={(e) => {
                        setCloudToken(e.target.value);
                        change({ cloud_token: e.target.value });
                      }}
                      onBlur={() => void flushSettings()}
                    />
                    <button
                      className="ghost"
                      onClick={() => setReveal((v) => ({ ...v, cloud: !v.cloud }))}
                    >
                      {reveal.cloud ? "隐藏" : "显示"}
                    </button>
                  </div>
                  <div className="hintline">
                    与远端启动时的 NANOHERMES_DAEMON_TOKEN 一致；留空表示服务端未开启鉴权。
                  </div>
                </div>
              </>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>模型</h3>
              <p>
                可以保存多个模型服务（不同的地址或密钥），在输入区随时切换；每个任务会记住自己用的那个。
                「默认项」是新任务以及没指定模型的任务使用的。
              </p>
            </div>
            <div className="sb">
              {models.map((m, i) => {
                const p = mprobe[m.id];
                const on = m.id === activeModel;
                return (
                  <div className="mcard" key={m.id}>
                    <div className="mchead">
                      <input
                        className="mnameinput"
                        value={m.name}
                        spellCheck={false}
                        placeholder="给这个模型起个名字"
                        onChange={(e) => editModel(i, { name: e.target.value })}
                        onBlur={() => void flushSettings()}
                      />
                      <button
                        className={"ghost" + (on ? " on" : "")}
                        title="新任务与未指定模型的任务都使用这个模型"
                        onClick={() => makeDefault(m.id)}
                      >
                        <Icon name={on ? "check" : "spark"} />
                        {on ? "默认项" : "设为默认"}
                      </button>
                      <button
                        className="cbtn"
                        title={models.length <= 1 ? "至少要保留一个模型" : "删除这个模型"}
                        disabled={models.length <= 1}
                        onClick={() => removeModel(m.id)}
                      >
                        <Icon name="trash" />
                      </button>
                    </div>
                    <div className="field">
                      <label>API Key</label>
                      <div className="row">
                        <input
                          type={reveal[m.id] ? "text" : "password"}
                          value={m.api_key}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder="sk-..."
                          onChange={(e) => editModel(i, { api_key: e.target.value })}
                          onBlur={() => void flushSettings()}
                        />
                        <button
                          className="ghost"
                          onClick={() => setReveal((v) => ({ ...v, [m.id]: !v[m.id] }))}
                        >
                          {reveal[m.id] ? "隐藏" : "显示"}
                        </button>
                      </div>
                    </div>
                    <div className="field">
                      <label>Base URL</label>
                      <div className="row">
                        <input
                          type="text"
                          value={m.base_url}
                          spellCheck={false}
                          placeholder="https://api.openai.com/v1"
                          onChange={(e) => editModel(i, { base_url: e.target.value })}
                          onBlur={() => void flushSettings()}
                        />
                        <button
                          className="ghost"
                          disabled={!m.base_url.trim()}
                          onClick={() => void testModel(m)}
                        >
                          测试连接
                        </button>
                      </div>
                    </div>
                    <div className="field">
                      <label>模型</label>
                      <input
                        type="text"
                        value={m.model}
                        spellCheck={false}
                        placeholder="gpt-4o-mini"
                        onChange={(e) => editModel(i, { model: e.target.value })}
                        onBlur={() => void flushSettings()}
                      />
                    </div>
                    <div
                      className="hintline"
                      style={p ? { color: p.ok ? "var(--ok)" : "var(--danger)" } : undefined}
                    >
                      {p
                        ? p.t
                        : "「测试连接」用当前 Key 请求一次 /models 校验地址与密钥，不消耗 token。"}
                    </div>
                  </div>
                );
              })}
              <button className="ghost addmodel" onClick={addModel}>
                <Icon name="plus" /> 添加模型
              </button>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>人设</h3>
              <p>
                人设会作为附加指令拼在内置系统提示词之后，只影响本机任务；不同的任务可以各选各的。
                「默认项」不附加任何指令，行为与之前完全一致。
              </p>
            </div>
            <div className="sb">
              {personas.map((p, i) => {
                const on = p.id === activePersona;
                return (
                  <div className="mcard" key={p.id}>
                    <div className="mchead">
                      <input
                        className="mnameinput"
                        value={p.name}
                        spellCheck={false}
                        placeholder="给这个人设起个名字"
                        onChange={(e) => editPersona(i, { name: e.target.value })}
                        onBlur={() => void flushSettings()}
                      />
                      <button
                        className={"ghost" + (on ? " on" : "")}
                        title="新任务与未指定人设的任务都使用这个人设"
                        onClick={() => makeDefaultPersona(p.id)}
                      >
                        <Icon name={on ? "check" : "user"} />
                        {on ? "当前默认" : "设为默认"}
                      </button>
                      <button
                        className="cbtn"
                        title={personas.length <= 1 ? "至少要保留一个人设" : "删除这个人设"}
                        disabled={personas.length <= 1}
                        onClick={() => removePersona(p.id)}
                      >
                        <Icon name="trash" />
                      </button>
                    </div>
                    <div className="field">
                      <label>附加指令</label>
                      <textarea
                        className="ptext"
                        rows={3}
                        spellCheck={false}
                        value={p.prompt}
                        placeholder="留空表示不附加任何指令"
                        onChange={(e) => editPersona(i, { prompt: e.target.value })}
                        onBlur={() => void flushSettings()}
                      />
                    </div>
                    <div className="hintline">
                      例如：回答一律用中文，先给结论再给理由；改动会在发送下一条消息时生效。
                    </div>
                  </div>
                );
              })}
              <button className="ghost addmodel" onClick={addPersona}>
                <Icon name="plus" /> 添加人设
              </button>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>提示词片段</h3>
              <p>
                常用的短语或模板。在输入区点闪电图标即可把它们插入到当前输入末尾；片段只插入文本，
                本机与云端任务都能用。
              </p>
            </div>
            <div className="sb">
              {snippets.length === 0 && (
                <div className="hintline">还没有片段，点下面的按钮添加一条。</div>
              )}
              {snippets.map((x, i) => (
                <div className="mcard" key={x.id}>
                  <div className="mchead">
                    <input
                      className="mnameinput"
                      value={x.name}
                      spellCheck={false}
                      placeholder="片段名称"
                      onChange={(e) => editSnippet(i, { name: e.target.value })}
                      onBlur={() => void flushSettings()}
                    />
                    <button className="cbtn" title="删除这个片段" onClick={() => removeSnippet(x.id)}>
                      <Icon name="trash" />
                    </button>
                  </div>
                  <div className="field">
                    <label>内容</label>
                    <textarea
                      className="ptext"
                      rows={2}
                      spellCheck={false}
                      value={x.text}
                      placeholder="插入到输入框的文本"
                      onChange={(e) => editSnippet(i, { text: e.target.value })}
                      onBlur={() => void flushSettings()}
                    />
                  </div>
                </div>
              ))}
              <button className="ghost addmodel" onClick={addSnippet}>
                <Icon name="plus" /> 添加片段
              </button>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>工作区</h3>
              <p>Agent 读写文件、执行命令的项目根目录；预览与引用仅限该目录。每个任务也可以单独指定。</p>
            </div>
            <div className="sb">
              <div className="field">
                <label>默认工作区</label>
                <input
                  type="text"
                  value={workspace}
                  spellCheck={false}
                  placeholder="D:\project"
                  onChange={(e) => {
                    setWorkspace(e.target.value);
                    change({ workspace: e.target.value });
                  }}
                  onBlur={() => void flushSettings()}
                />
                <div className="hintline">
                  新建任务时的默认目录；本机任务与云端任务都会用到（云端任务指的是远端机器上的路径）。
                </div>
              </div>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>权限模式</h3>
              <p>本机任务使用的权限模式；云端任务的权限由远端 daemon 的 --permission-mode 启动参数决定。</p>
            </div>
            <div className="sb">
              <div
                className="modepick"
                role="radiogroup"
                aria-label="权限模式"
                onKeyDown={(e) => radioGroupKeys(e)}
              >
                {MODES.map(([value, name, desc]) => (
                  <button
                    key={value}
                    role="radio"
                    aria-checked={st.mode === value}
                    tabIndex={st.mode === value ? 0 : -1}
                    className={st.mode === value ? "on" : ""}
                    onClick={() => void setMode(value)}
                  >
                    <span>
                      <span className="k">{name}</span>
                      <span className="v">{desc}</span>
                    </span>
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section className="sect">
            <div className="sh">
              <h3>高级</h3>
              <p>本机任务的运行参数，只影响本机执行；云端任务的行为由远端 daemon 的启动参数决定。</p>
            </div>
            <div className="sb">
              <div className="toggleline">
                <div>
                  <span className="k">深度思考</span>
                  <span className="v">让支持推理的模型先思考再作答（更慢，通常更稳）。</span>
                </div>
                <button
                  className={"ghost" + (thinking ? " on" : "")}
                  title="深度思考开关"
                  aria-pressed={thinking}
                  onClick={() => setToggle("thinking", !thinking)}
                >
                  {thinking ? "已开启" : "已关闭"}
                </button>
              </div>
              <div className="toggleline">
                <div>
                  <span className="k">读取项目指南</span>
                  <span className="v">把工作区里的 AGENTS.md / NANOHERMES.md 读进系统提示词。</span>
                </div>
                <button
                  className={"ghost" + (projectDocs ? " on" : "")}
                  title="读取项目指南开关"
                  aria-pressed={projectDocs}
                  onClick={() => setToggle("project_docs", !projectDocs)}
                >
                  {projectDocs ? "已开启" : "已关闭"}
                </button>
              </div>
              <div className="toggleline">
                <div>
                  <span className="k">长期记忆</span>
                  <span className="v">
                    对话结束后自动沉淀经验、偏好与纠正，新会话按相关度检索注入；关闭后不再提炼，已有记忆保留。
                  </span>
                </div>
                <button
                  className={"ghost" + (memory ? " on" : "")}
                  title="长期记忆开关"
                  aria-pressed={memory}
                  onClick={() => setToggle("memory", !memory)}
                >
                  {memory ? "已开启" : "已关闭"}
                </button>
              </div>
              <div className="numgrid">
                <div className="field">
                  <label>温度</label>
                  <input
                    type="number"
                    min={0}
                    max={2}
                    step={0.1}
                    value={numberDraft.temperature}
                    onChange={(e) => setNumber("temperature", e.target.value)}
                    onBlur={() => settleNumber("temperature")}
                  />
                  <div className="hintline">0 更稳定，越大越发散。</div>
                </div>
                <div className="field">
                  <label>最大迭代</label>
                  <input
                    type="number"
                    min={1}
                    max={200}
                    value={numberDraft.max_iterations}
                    onChange={(e) => setNumber("max_iterations", e.target.value)}
                    onBlur={() => settleNumber("max_iterations")}
                  />
                  <div className="hintline">一轮里模型与工具往返的上限。</div>
                </div>
                <div className="field">
                  <label>最大输出 Tokens</label>
                  <input
                    type="number"
                    min={1}
                    max={200000}
                    value={numberDraft.max_tokens}
                    onChange={(e) => setNumber("max_tokens", e.target.value)}
                    onBlur={() => settleNumber("max_tokens")}
                  />
                  <div className="hintline">单次回复允许生成的上限。</div>
                </div>
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

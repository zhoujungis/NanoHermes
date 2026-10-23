/**
 * store 的测试。
 *
 * store.ts 是模块级单例，测试通过 vi.resetModules() 每个用例拿一份全新的状态；
 * 同时把 ./tauri 桥接层替换成假实现，让这些用例只验证状态迁移本身，不碰真实 IPC。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ModelProfile, Persona, Settings, Task } from "./store";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(async (..._args: unknown[]) => undefined as unknown),
  listenMock: vi.fn(async () => () => {}),
}));

vi.mock("./tauri", () => ({
  inTauri: false,
  appWindow: () => null,
  invoke: invokeMock,
  listen: listenMock,
  errText: (e: unknown) =>
    typeof e === "string" ? e : e instanceof Error ? e.message : String(e),
}));

async function freshStore() {
  vi.resetModules();
  return await import("./store");
}

function settings(patch: Partial<Settings> = {}): Settings {
  return {
    api_key: "",
    base_url: "",
    model: "",
    models: [],
    active_model: "",
    personas: [],
    active_persona: "",
    snippets: [],
    workspace: "",
    mode: "default",
    location: "local",
    cloud_url: "",
    cloud_token: "",
    thinking: false,
    project_docs: true,
    memory: true,
    temperature: 0,
    max_iterations: 40,
    max_tokens: 4096,
    ...patch,
  };
}

const prof = (id: string, name: string, model = "", key = "k"): ModelProfile => ({
  id,
  name,
  base_url: "https://" + id + "/v1",
  api_key: key,
  model,
});

beforeEach(() => {
  localStorage.clear();
  invokeMock.mockClear();
  listenMock.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("纯函数", () => {
  it("isCloud 只认显式的云端设置", async () => {
    const { isCloud } = await freshStore();
    expect(isCloud(null)).toBe(false);
    expect(isCloud(settings())).toBe(false);
    expect(isCloud(settings({ location: "cloud" }))).toBe(true);
  });

  it("taskIsCloud 只看任务自身的执行位置", async () => {
    const { taskIsCloud } = await freshStore();
    expect(taskIsCloud(null)).toBe(false);
    expect(taskIsCloud({ location: "local" } as Task)).toBe(false);
    expect(taskIsCloud({ location: "cloud" } as Task)).toBe(true);
  });

  it("titleFrom 压缩空白、超长截断", async () => {
    const { titleFrom } = await freshStore();
    expect(titleFrom("")).toBe("新任务");
    expect(titleFrom("   ")).toBe("新任务");
    expect(titleFrom("  多个   空格  ")).toBe("多个 空格");
    const long = "一".repeat(40);
    const t = titleFrom(long);
    expect(t.endsWith("…")).toBe(true);
    expect(t.length).toBe(23);
  });

  it("resolveModel 依次尝试显式 id、默认项、第一项", async () => {
    const { resolveModel } = await freshStore();
    const s = settings({ models: [prof("a", "甲"), prof("b", "乙")], active_model: "a" });

    expect(resolveModel(s, "b")?.id).toBe("b");
    expect(resolveModel(s, "")?.id).toBe("a");
    expect(resolveModel(s, "不存在")?.id).toBe("a");
    expect(resolveModel({ ...s, active_model: "" }, null)?.id).toBe("a");
    expect(resolveModel(settings({ models: [] }), null)).toBeNull();
    expect(resolveModel(null, "a")).toBeNull();
  });

  it("modelLabel 退回名字、模型名与默认文案", async () => {
    const { modelLabel } = await freshStore();
    const s = settings({
      models: [prof("a", "快的", "gpt-4o-mini"), prof("b", "", "claude-3-5")],
      active_model: "a",
    });
    expect(modelLabel(s, "a")).toBe("快的");
    expect(modelLabel(s, "b")).toBe("claude-3-5");
    expect(modelLabel(s, "不存在")).toBe("快的");
    expect(modelLabel(settings({ models: [] }), null)).toBe("默认模型");
  });
});

const per = (id: string, name: string, prompt = "x"): Persona => ({ id, name, prompt });

describe("人设", () => {
  it("resolvePersona 依次尝试显式 id、默认项、第一项", async () => {
    const { resolvePersona } = await freshStore();
    const s = settings({ personas: [per("a", "甲"), per("b", "乙")], active_persona: "a" });

    expect(resolvePersona(s, "b")?.id).toBe("b");
    expect(resolvePersona(s, "")?.id).toBe("a");
    expect(resolvePersona(s, "不存在")?.id).toBe("a");
    expect(resolvePersona({ ...s, active_persona: "" }, null)?.id).toBe("a");
    expect(resolvePersona(settings({ personas: [] }), null)).toBeNull();
    expect(resolvePersona(null, "a")).toBeNull();
  });

  it("personaLabel 退回默认文案，personaIsCustom 只看附加指令", async () => {
    const { personaLabel, personaIsCustom, resolvePersona } = await freshStore();
    const s = settings({
      personas: [per("a", "审查员", "你是审查员"), per("b", "", "  ")],
      active_persona: "a",
    });
    expect(personaLabel(s, "a")).toBe("审查员");
    expect(personaLabel(s, "b")).toBe("默认人设");
    expect(personaLabel(settings({ personas: [] }), null)).toBe("默认人设");
    expect(personaIsCustom(resolvePersona(s, "a"))).toBe(true);
    // 只有空白的附加指令不算自定义人设，后端不会追加任何内容。
    expect(personaIsCustom(resolvePersona(s, "b"))).toBe(false);
    expect(personaIsCustom(null)).toBe(false);
  });

  it("新建任务绑定当前默认人设，切换人设会用新档案重绑", async () => {
    const s = await freshStore();
    s.autosaveSettings(
      settings({ personas: [per("a", "甲"), per("b", "乙")], active_persona: "b" }),
    );
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;
    expect(s.getState().tasks[0].persona).toBe("b");
    invokeMock.mockClear();

    await s.setTaskPersona(id, "a");

    expect(s.getState().tasks[0].persona).toBe("a");
    expect(invokeMock).toHaveBeenCalledWith("open_task", {
      id,
      workspace: "C:/ws",
      model: null,
      persona: "a",
    });
  });
});

describe("置顶与片段", () => {
  it("togglePin 来回切换任务的置顶状态", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;
    expect(s.getState().tasks[0].pinned).toBeFalsy();

    s.togglePin(id);
    expect(s.getState().tasks[0].pinned).toBe(true);
    s.togglePin(id);
    expect(s.getState().tasks[0].pinned).toBe(false);
  });

  it("insertSnippet 追加到输入末尾，空内容被忽略", async () => {
    const s = await freshStore();
    s.setPrompt("先做 A");
    s.insertSnippet("再做 B");
    expect(s.getState().prompt).toBe("先做 A\n再做 B");

    s.setPrompt("");
    s.insertSnippet("  只留这一段  ");
    expect(s.getState().prompt).toBe("只留这一段");

    s.insertSnippet("   ");
    expect(s.getState().prompt).toBe("只留这一段");
  });
});

describe("输入区状态", () => {
  it("prompt / 筛选 / 页签各自独立更新", async () => {
    const s = await freshStore();
    s.setPrompt("你好");
    s.setSearch("foo");
    s.setFilter("w");
    s.setPanelTab("tree");

    const st = s.getState();
    expect(st.prompt).toBe("你好");
    expect(st.search).toBe("foo");
    expect(st.filter).toBe("w");
    expect(st.panelTab).toBe("tree");
  });

  it("引用文件自动去重", async () => {
    const s = await freshStore();
    s.addRef("src/a.ts");
    s.addRef("src/a.ts");
    expect(s.getState().refs).toEqual(["src/a.ts"]);
    s.removeRef("src/a.ts");
    expect(s.getState().refs).toEqual([]);
  });
});

describe("提示条", () => {
  it("默认只是说明，成功与失败各自带等级", async () => {
    const s = await freshStore();
    s.toast("普通提示");
    expect(s.getState().toast).toBe("普通提示");
    expect(s.getState().toastKind).toBe("info");

    s.toast("已保存", "ok");
    expect(s.getState().toast).toBe("已保存");
    expect(s.getState().toastKind).toBe("ok");

    s.toast("保存失败", "err");
    expect(s.getState().toastKind).toBe("err");
  });

  it("到时间后自动收起", async () => {
    vi.useFakeTimers();
    const s = await freshStore();
    s.toast("稍纵即逝");
    expect(s.getState().toast).toBe("稍纵即逝");

    vi.advanceTimersByTime(2100);
    expect(s.getState().toast).toBe("");
  });
});

describe("任务", () => {
  it("新建本机任务时若没有工作区就弹出文件夹选择器", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "");

    const st = s.getState();
    expect(st.tasks.length).toBe(1);
    expect(st.tasks[0].location).toBe("local");
    expect(st.activeId).toBe(st.tasks[0].id);
    expect(st.folderPick?.forTask).toBe(st.tasks[0].id);
  });

  it("新建云端任务不会要求本地工作区", async () => {
    const s = await freshStore();
    await s.createTask(true, "cloud");

    const st = s.getState();
    expect(st.tasks[0].location).toBe("cloud");
    expect(st.folderPick).toBeNull();
  });

  it("重命名会去掉首尾空白，空标题被忽略", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;

    s.renameTask(id, "  我的任务 ");
    expect(s.getState().tasks[0].title).toBe("我的任务");

    s.renameTask(id, "   ");
    expect(s.getState().tasks[0].title).toBe("我的任务");
  });

  it("切换执行位置会更新任务并让后端重绑", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;
    invokeMock.mockClear();

    await s.setTaskLocation(id, "cloud");

    expect(s.getState().tasks[0].location).toBe("cloud");
    expect(invokeMock).toHaveBeenCalledWith("open_task", {
      id,
      workspace: "C:/ws",
      model: null,
      persona: null,
    });
  });

  it("切换模型会更新任务并让后端用新档案重绑", async () => {
    const s = await freshStore();
    s.autosaveSettings(
      settings({ models: [prof("a", "甲"), prof("b", "乙")], active_model: "a" }),
    );
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;
    expect(s.getState().tasks[0].model).toBe("a");
    invokeMock.mockClear();

    await s.setTaskModel(id, "b");

    expect(s.getState().tasks[0].model).toBe("b");
    expect(invokeMock).toHaveBeenCalledWith("open_task", {
      id,
      workspace: "C:/ws",
      model: "b",
      persona: null,
    });
  });

  it("新建任务固定用当前的默认模型，之后改默认项不动老任务", async () => {
    const s = await freshStore();
    s.autosaveSettings(
      settings({ models: [prof("a", "甲"), prof("b", "乙")], active_model: "a" }),
    );
    await s.createTask(true, "local", "C:/a");
    const first = s.getState().tasks[0].id;
    expect(s.getState().tasks[0].model).toBe("a");

    // 把默认改成 b：已经建好的任务保持 a，新任务才是 b。
    s.autosaveSettings(
      settings({ models: [prof("a", "甲"), prof("b", "乙")], active_model: "b" }),
    );
    await s.createTask(true, "local", "C:/b");

    const [one, two] = s.getState().tasks;
    expect(one.id).toBe(first);
    expect(one.model).toBe("a");
    expect(two.model).toBe("b");
  });

  it("设置工作区会去掉空白并落回任务", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "C:/ws");
    const id = s.getState().tasks[0].id;

    await s.setTaskWorkspace(id, "  D:/proj  ");
    expect(s.getState().tasks[0].workspace).toBe("D:/proj");

    await s.setTaskWorkspace(id, "   ");
    expect(s.getState().tasks[0].workspace).toBe("D:/proj");
  });

  it("删除当前任务后回落到相邻任务", async () => {
    const s = await freshStore();
    await s.createTask(true, "local", "C:/a");
    await s.createTask(true, "local", "C:/b");
    const [a, b] = s.getState().tasks.map((t) => t.id);
    expect(s.getState().activeId).toBe(b);

    await s.deleteTask(b);

    expect(s.getState().tasks.map((t) => t.id)).toEqual([a]);
    expect(s.getState().activeId).toBe(a);
  });
});

describe("设置", () => {
  it("切换模式同时更新 state.mode 与 settings.mode", async () => {
    const s = await freshStore();
    s.autosaveSettings(settings());
    invokeMock.mockClear();

    await s.setMode("plan");

    expect(s.getState().mode).toBe("plan");
    expect(s.getState().settings?.mode).toBe("plan");
    expect(invokeMock).toHaveBeenCalledWith("set_mode", { mode: "plan" });
  });

  it("自动保存写内存状态并把状态标为 pending", async () => {
    const s = await freshStore();
    s.autosaveSettings(settings({ model: "gpt-4o-mini" }));

    const st = s.getState();
    expect(st.settings?.model).toBe("gpt-4o-mini");
    expect(st.mode).toBe("default");
    expect(st.settingsSync).toBe("pending");
  });
});

describe("输入历史", () => {
  it("入栈去空白、连续重复去重", async () => {
    const s = await freshStore();
    s.pushHistory("  第一条  ");
    s.pushHistory("第一条"); // 连续重复，不入栈
    s.pushHistory("第二条");
    expect(s.inputHistory()).toEqual(["第一条", "第二条"]);
  });

  it("空白与空串不入栈", async () => {
    const s = await freshStore();
    s.pushHistory("   ");
    s.pushHistory("");
    expect(s.inputHistory()).toEqual([]);
  });

  it("超出上限时丢最旧的", async () => {
    const s = await freshStore();
    for (let i = 0; i < 60; i++) s.pushHistory(`条目 ${i}`);
    const hist = s.inputHistory();
    expect(hist).toHaveLength(50);
    expect(hist[0]).toBe("条目 10"); // 0~9 被挤掉
    expect(hist[49]).toBe("条目 59");
  });

  it("send() 只把输入框发出的文本记进历史，重试不记", async () => {
    const s = await freshStore();
    s.autosaveSettings(settings());
    await s.createTask(true, "local", "C:/ws");
    s.setPrompt("帮我看看这个 bug");

    await s.send();
    expect(s.inputHistory()).toEqual(["帮我看看这个 bug"]);
    expect(s.getState().prompt).toBe(""); // 发完清空输入框

    await s.send("重试的文本"); // 显式传参（重试）不入历史
    expect(s.inputHistory()).toEqual(["帮我看看这个 bug"]);
  });
});

describe("insertAt", () => {
  it("在光标处插入并聚焦输入框", async () => {
    const s = await freshStore();
    s.setPrompt("看 src/a.rs");
    const tick = s.getState().focusTick;

    s.insertAt(0, "@ref ");

    expect(s.getState().prompt).toBe("@ref 看 src/a.rs");
    expect(s.getState().focusTick).toBe(tick + 1);
  });

  it("位置越界时夹到合法范围", async () => {
    const s = await freshStore();
    s.setPrompt("abc");
    s.insertAt(99, "!");
    expect(s.getState().prompt).toBe("abc!");
    s.insertAt(-5, "?");
    expect(s.getState().prompt).toBe("?abc!");
  });
});

describe("流式实时文本的独立通道", () => {
  const DELTA = "nanohermes://delta";
  const TOOL = "nanohermes://tool";

  /** 起一轮挂起的发送：run_task 先不返回，增量由测试通过事件通道喂进去。 */
  async function startStream() {
    let resolveRun: (v: string) => void = () => {};
    let rejectRun: (e: unknown) => void = () => {};
    invokeMock.mockImplementation((cmd: unknown) =>
      cmd === "run_task"
        ? new Promise<string>((res, rej) => {
            resolveRun = res;
            rejectRun = rej;
          })
        : Promise.resolve(undefined),
    );
    const cbs: Record<string, (p: unknown) => void> = {};
    listenMock.mockImplementation(async (...args: unknown[]) => {
      cbs[String(args[0])] = args[1] as (p: unknown) => void;
      return () => {};
    });

    const s = await freshStore();
    s.autosaveSettings(settings());
    await s.createTask(true, "local", "C:/ws");
    s.setPrompt("讲讲这段代码");
    s.wireEvents();
    const done = s.send(); // 挂起在 run_task 上，流式态一直保持
    return {
      s,
      cbs,
      delta: (t: string) => cbs[DELTA](t),
      finish: (v = "") => resolveRun(v),
      fail: (m: string) => rejectRun(new Error(m)),
      done,
    };
  }

  /** 等过 40ms 的增量合并窗口。 */
  const tick = () => new Promise((r) => setTimeout(r, 70));

  it("增量期间只推独立通道，完全不写全局状态", async () => {
    const { s, delta } = await startStream();
    const snapshot = s.getState();
    const msgsBefore = snapshot.tasks.find((t) => t.id === snapshot.activeId)!.msgs;

    delta("第一段内容。\n\n");
    delta("第二段还在写");
    await tick();

    // 实时文本到了通道里……
    expect(s.getStreamText()).toBe("第一段内容。\n\n第二段还在写");
    // ……但全局状态对象身份没变，也就是这期间没有任何 commit：侧栏、结果区、
    // 顶栏、输入区都不会被每帧唤醒，虚拟列表的高度回测也不会每帧重排。
    // 这是「输出时整个界面卡住、鼠标都动不了」的根治点。
    expect(s.getState()).toBe(snapshot);

    // 消息里也不能逐帧写文本——流式期间 t 恒为空，直到这一步结束才定稿。
    const msgs = s.getState().tasks.find((t) => t.id === snapshot.activeId)!.msgs;
    expect(msgs).toBe(msgsBefore);
    expect(msgs[msgs.length - 1].t).toBe("");
    expect(msgs[msgs.length - 1].streaming).toBe(true);
  });

  it("阶段结束时把通道里的文本一次性写进消息，并清空通道", async () => {
    const { s, cbs, delta, finish, done } = await startStream();
    delta("先思考一下。\n\n");
    await tick();

    // 模型先调工具：这一步输出的文本定稿成一条消息，工具行接在后面。
    cbs[TOOL]({ name: "read_file", summary: "src/a.rs", output: "fn main() {}" });

    const t0 = s.getState().tasks.find((t) => t.id === s.getState().activeId)!;
    expect(t0.msgs.map((m) => m.k)).toEqual(["user", "ai", "tool"]);
    expect(t0.msgs[1].t).toBe("先思考一下。\n\n");
    expect(t0.msgs[1].streaming).toBe(false);
    expect(s.getStreamText()).toBe("");

    // 整轮收尾：第二段输出落成终态气泡，通道清空、忙碌标记解除。
    delta("结论是这样。");
    await tick();
    finish("");
    await done;

    const msgs = s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs;
    const last = msgs[msgs.length - 1];
    expect(last.k).toBe("ai");
    expect(last.t).toBe("结论是这样。");
    expect(last.streaming).toBe(false);
    expect(s.getStreamText()).toBe("");
    expect(s.getState().busy).toBe(false);
  });

  it("没有增量时退回 run_task 的返回值（兼容非流式后端）", async () => {
    const { s, finish, done } = await startStream();
    finish("整段一次性返回的答案");
    await done;

    const msgs = s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs;
    const last = msgs[msgs.length - 1];
    expect(last.k).toBe("ai");
    expect(last.t).toBe("整段一次性返回的答案");
    expect(last.streaming).toBe(false);
    expect(s.getStreamText()).toBe("");
  });

  it("停止请求一直保持到本轮收尾，期间不能重复发送", async () => {
    const { s, finish, done } = await startStream();

    await s.stop();
    // 停止只在步骤边界生效：本轮还在跑，标记必须一直亮着，界面才有东西可显示。
    expect(s.getState().busy).toBe(true);
    expect(s.getState().stopping).toBe(true);

    // 等待期间再点一次停止：不再发第二条命令，也不会把状态改回去。
    await s.stop();
    expect(invokeMock.mock.calls.filter((c) => c[0] === "stop_task")).toHaveLength(1);

    // 等待期间按 Enter：不再静默丢弃，而是加入队列（停止后不会自动续发）。
    s.setPrompt("运行中再问一句");
    await s.send();
    expect(s.getState().toast).toContain("已加入队列");
    expect(s.getState().prompt).toBe("");
    const queued = s.getState().tasks.find((t) => t.id === s.getState().activeId)!;
    expect(queued.queue?.map((q) => q.text)).toEqual(["运行中再问一句"]);

    finish("");
    await done;
    expect(s.getState().busy).toBe(false);
    expect(s.getState().stopping).toBe(false);
    // 主动停止后队列不自动续跑，留给用户决定何时继续。
    expect(
      s.getState().tasks.find((t) => t.id === s.getState().activeId)!.queue,
    ).toHaveLength(1);
    // 收尾时给一次明确回执，用户才知道停止真的生效了。
    expect(s.getState().toast).toBe("任务已停止");
  });

  it("停止后连接断开按停止处理，不冒红色错误气泡", async () => {
    const { s, delta, fail, done } = await startStream();
    delta("已经写了一半");
    await tick();

    await s.stop();
    // 后端察觉停止后主动断开，客户端看到的是读取失败；这不是故障。
    fail("读取云端流失败：connection closed");
    await done;

    const msgs = s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs;
    expect(msgs.some((m) => m.err)).toBe(false);
    expect(msgs.some((m) => m.k === "note" && (m.t || "").includes("运行中断"))).toBe(false);
    expect(msgs.some((m) => m.k === "ai" && m.t === "已经写了一半")).toBe(true);
    expect(s.getState().busy).toBe(false);
    expect(s.getState().stopping).toBe(false);
    expect(s.getState().toast).toBe("任务已停止");
  });

  it("运行中继续发送会入队，本轮正常结束后按顺序自动续发", async () => {
    const { s, finish, done } = await startStream();

    s.setPrompt("第二轮的问题");
    await s.send();
    // 入队：输入框清空、队列留一条，此时还没有发出第二次请求。
    expect(s.getState().prompt).toBe("");
    expect(s.getState().toast).toContain("已加入队列");
    expect(
      s.getState().tasks.find((t) => t.id === s.getState().activeId)!.queue?.map((q) => q.text),
    ).toEqual(["第二轮的问题"]);
    expect(invokeMock.mock.calls.filter((c) => c[0] === "run_task")).toHaveLength(1);

    // 第一轮正常收尾：队列被取走并自动发出第二轮。
    finish("第一轮回答");
    await done;
    await new Promise((r) => setTimeout(r, 30));

    const task = s.getState().tasks.find((t) => t.id === s.getState().activeId)!;
    expect(task.queue ?? []).toEqual([]);
    expect(task.msgs.filter((m) => m.k === "user")).toHaveLength(2);
    expect(task.msgs.some((m) => m.k === "ai" && m.t === "第一轮回答")).toBe(true);
    expect(invokeMock.mock.calls.filter((c) => c[0] === "run_task")).toHaveLength(2);
    expect(s.getState().busy).toBe(true);
  });

  it("折叠长回答；流式中的消息不允许折叠", async () => {
    const s = await freshStore();
    s.autosaveSettings(settings());
    await s.createTask(true, "local", "C:/ws");

    const long = "这是一段很长的回答。".repeat(40);
    s.pushMsg({ k: "ai", t: long, ts: Date.now() });
    const task = s.getState().tasks.find((t) => t.id === s.getState().activeId)!;
    const index = task.msgs.length - 1;

    s.toggleFold(index);
    expect(
      s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs[index].fold,
    ).toBe(true);

    s.toggleFold(index);
    expect(
      s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs[index].fold,
    ).toBe(false);

    s.pushMsg({ k: "ai", t: "还在写", ts: Date.now(), streaming: true });
    const last = s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs.length - 1;
    s.toggleFold(last);
    expect(
      s.getState().tasks.find((t) => t.id === s.getState().activeId)!.msgs[last].fold,
    ).toBeUndefined();
  });
});

describe("持久化", () => {
  const LS_KEY = "nanohermes.state.v1";

  it("流式中途落盘会把实时文本写进去，而不是留下一个转圈的空气泡", async () => {
    const s = await freshStore();
    const h = s.devBench();
    h.delta("已经生成的内容");

    // 关闭窗口会同步触发 beforeunload → save()，此时增量还没写回状态。
    window.dispatchEvent(new Event("beforeunload"));

    const saved = JSON.parse(localStorage.getItem(LS_KEY) || "{}");
    const msgs = saved.tasks[0].msgs as Array<Record<string, unknown>>;
    const ai = msgs.find((m) => m.k === "ai")!;
    expect(ai.t).toBe("已经生成的内容");
    expect(ai.streaming).toBe(false);
    h.end();
  });

  it("加载旧状态时清掉 streaming 标记和空气泡", async () => {
    localStorage.setItem(
      LS_KEY,
      JSON.stringify({
        activeId: "t1",
        tasks: [
          {
            id: "t1",
            title: "旧任务",
            createdAt: 1,
            updatedAt: 1,
            msgs: [
              { k: "user", t: "你好", ts: 1 },
              { k: "ai", t: "", ts: 2, streaming: true },
              { k: "ai", t: "完整回答", ts: 3, streaming: true },
            ],
            files: [],
            location: "local",
            workspace: "",
            model: "",
            persona: "",
          },
        ],
      }),
    );

    const s = await freshStore();
    await s.boot();

    const msgs = s.getState().tasks.find((t) => t.id === "t1")!.msgs;
    expect(msgs.some((m) => m.streaming)).toBe(false);
    expect(msgs.some((m) => m.k === "ai" && !m.t)).toBe(false);
    expect(msgs.some((m) => m.t === "完整回答")).toBe(true);
  });
});

/**
 * 端到端流程测试：真实 store + 真实组件，只把 Tauri IPC 桥 mock 掉。
 *
 * 覆盖最近四项改动的完整用户流程：
 * 1. 超长会话虚拟滚动：窗口化渲染、滚动换窗、查找命中不受窗口影响、短会话全量
 * 2. 结果区「变更」视图语法级着色 diff：行级增删分类 + 行内词法着色 + 降级分支
 * 3. 快捷键提示面板：Ctrl+/ 与命令面板两条入口、Esc 关闭
 * 4. 设置页键盘遍历：radiogroup 语义、roving tabindex、方向键切换
 *
 * jsdom 没有真实布局：消息实测高度恒为 0，窗口计算会一直用估算高度（96px），
 * 因此窗口位置在测试里是确定的，可以直接断言哪些下标被挂载。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(async (..._args: unknown[]) => undefined as unknown),
  listenMock: vi.fn(async (..._args: unknown[]) => (() => {}) as () => void),
}));

vi.mock("./lib/tauri", () => ({
  inTauri: false,
  appWindow: () => null,
  invoke: invokeMock,
  listen: listenMock,
  errText: (e: unknown) =>
    typeof e === "string" ? e : e instanceof Error ? e.message : String(e),
}));

// jsdom 的 Range 缺 getBoundingClientRect / getClientRects（真实 WebView 有），
// 查找定位会调用；这里补空实现，只影响测试环境。
if (!Range.prototype.getBoundingClientRect) {
  Range.prototype.getBoundingClientRect = () => new DOMRect(0, 0, 0, 0);
}
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
}

/** 全新模块图（store 是模块级单例）+ 一个已建好的本机任务。 */
async function freshApp() {
  vi.resetModules();
  const store = await import("./lib/store");
  const { default: App } = await import("./App");
  await store.createTask(true, "local", "C:/ws");
  return { store, App };
}

/** 往当前任务造 n 条用户消息，文本里带下标便于查找命中。 */
function seed(store: Awaited<ReturnType<typeof freshApp>>["store"], n: number): void {
  for (let i = 0; i < n; i++) store.pushMsg({ k: "user", t: `第 ${i} 条消息`, ts: 1000 + i });
}

beforeEach(() => {
  localStorage.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async () => undefined as unknown);
  listenMock.mockClear();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("虚拟滚动（端到端）", () => {
  it("超长会话只挂载渲染窗口，远处消息不进 DOM", async () => {
    const { store, App } = await freshApp();
    seed(store, 120);
    const { container } = render(<App />);

    const slots = container.querySelectorAll(".m-slot[data-si]");
    expect(slots.length).toBeGreaterThan(0);
    expect(slots.length).toBeLessThan(30);
    // 顶部窗口在起点，末尾消息在窗口外。
    expect(container.querySelector('[data-si="0"]')).not.toBeNull();
    expect(container.querySelector('[data-si="119"]')).toBeNull();
  });

  it("滚动到深处后渲染窗口跟着移动", async () => {
    const { store, App } = await freshApp();
    seed(store, 120);
    const { container } = render(<App />);
    const el = container.querySelector(".scroller") as HTMLElement;

    // 5000px ÷ 估算高度 96px ≈ 第 52 条附近；窗口=46..58。
    el.scrollTop = 5000;
    fireEvent.scroll(el);

    await waitFor(() => {
      expect(container.querySelector('[data-si="52"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-si="0"]')).toBeNull();
    expect(container.querySelector('[data-si="58"]')).not.toBeNull();
    expect(container.querySelector('[data-si="59"]')).toBeNull();
  });

  it("查找命中所在消息被强制渲染，虚拟滚动不影响定位", async () => {
    const { store, App } = await freshApp();
    seed(store, 120);
    const { container } = render(<App />);
    const el = container.querySelector(".scroller") as HTMLElement;
    el.scrollTop = 5000;
    fireEvent.scroll(el);
    await waitFor(() => expect(container.querySelector('[data-si="52"]')).not.toBeNull());

    // Ctrl+F 打开查找，命中只出现在第 100 条（远在窗口之外）。
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    fireEvent.change(screen.getByPlaceholderText("在对话中查找"), {
      target: { value: "第 100 条" },
    });

    await waitFor(() => {
      expect(container.querySelector('[data-mi="100"]')).not.toBeNull();
    });
    // 无关的窗口外消息仍然不渲染，extra 只拉命中消息。
    expect(container.querySelector('[data-mi="90"]')).toBeNull();
  });

  it("短会话全量渲染，行为与普通列表一致", async () => {
    const { store, App } = await freshApp();
    seed(store, 30);
    const { container } = render(<App />);
    expect(container.querySelectorAll(".m-slot[data-si]")).toHaveLength(30);
  });
});

describe("语法级着色 diff（端到端）", () => {
  const DIFF = [
    "diff --git a/src/main.rs b/src/main.rs",
    "--- a/src/main.rs",
    "+++ b/src/main.rs",
    "@@ -1,2 +1,2 @@",
    " fn main() {",
    "-    let x = 1;",
    "+    let x = 2;",
    " }",
  ].join("\n");

  it("「变更」视图按行分类增删，正文做词法着色，标记单独成段", async () => {
    const { store } = await freshApp();
    const { ResultsPanel } = await import("./components/ResultsPanel");
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      const cmd = args[0];
      if (cmd === "read_file_preview") return "fn main() {}";
      if (cmd === "file_diff") return { repo: true, text: DIFF };
      return undefined;
    });
    const { container } = render(<ResultsPanel hidden={false} onCollapse={() => {}} />);
    await store.openPreview("C:/ws/src/main.rs");
    fireEvent.click(screen.getByRole("button", { name: "变更" }));

    await waitFor(() => {
      expect(container.querySelector(".dl.d-add")).not.toBeNull();
    });
    // 行级分类：hunk 头、删除行、新增行、上下文行各归各类。
    expect(container.querySelector(".dl.d-hunk")).not.toBeNull();
    expect(container.querySelector(".dl.d-del")).not.toBeNull();
    expect(container.querySelectorAll(".dl.d-ctx").length).toBeGreaterThan(0);
    // 行内词法着色：关键字（let）各归各色，而不是整行一个颜色。
    expect(container.querySelector(".d-add .hljs-keyword")).not.toBeNull();
    // 行首 +/- 标记压暗单独成段。
    expect(container.querySelector(".d-add .dm")?.textContent).toBe("+");
    expect(container.querySelector(".d-del .dm")?.textContent).toBe("-");
  });

  it("没有差异时如实提示", async () => {
    const { store } = await freshApp();
    const { ResultsPanel } = await import("./components/ResultsPanel");
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      const cmd = args[0];
      if (cmd === "read_file_preview") return "fn main() {}";
      if (cmd === "file_diff") return { repo: true, text: "" };
      return undefined;
    });
    render(<ResultsPanel hidden={false} onCollapse={() => {}} />);
    await store.openPreview("C:/ws/src/main.rs");
    fireEvent.click(screen.getByRole("button", { name: "变更" }));

    await waitFor(() => {
      expect(screen.getByText("相对最近一次提交没有差异（新文件或已提交）。")).toBeInTheDocument();
    });
  });
});

describe("快捷键提示面板（端到端）", () => {
  it("Ctrl+/ 呼出面板，Esc 关闭", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: "/", ctrlKey: true });
    const dialog = screen.getByRole("dialog", { name: "键盘快捷键" });
    // 清单里既有全局项也有分组说明。
    expect(dialog).toHaveTextContent("打开命令面板");
    expect(dialog).toHaveTextContent("会话内查找");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "键盘快捷键" })).toBeNull();
  });

  it("Ctrl+/ 再按一次收起", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: "/", ctrlKey: true });
    expect(screen.getByRole("dialog", { name: "键盘快捷键" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "/", ctrlKey: true });
    expect(screen.queryByRole("dialog", { name: "键盘快捷键" })).toBeNull();
  });

  it("命令面板 →「键盘快捷键」也能打开", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    fireEvent.change(screen.getByPlaceholderText("搜索命令…"), {
      target: { value: "快捷键" },
    });
    fireEvent.click(screen.getByText("键盘快捷键"));

    expect(screen.getByRole("dialog", { name: "键盘快捷键" })).toBeInTheDocument();
  });
});

describe("设置页键盘遍历（端到端）", () => {
  it("Ctrl+, 打开设置，选项组 roving tabindex + 方向键切换", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const group = screen.getByRole("radiogroup", { name: "界面主题" });
    const items = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));

    // roving tabindex：只有选中项在 Tab 序里。
    expect(items.filter((el) => el.tabIndex === 0)).toHaveLength(1);
    const checkedBefore = items.findIndex((el) => el.classList.contains("on"));

    items[checkedBefore].focus();
    fireEvent.keyDown(group, { key: "ArrowRight" });

    const checkedAfter = items.findIndex((el) => el.classList.contains("on"));
    expect(checkedAfter).not.toBe(checkedBefore);
    expect(items[checkedAfter]).toHaveFocus();
    // 选中即生效：<html data-theme> 跟着变。
    expect(document.documentElement.dataset.theme).toBeTruthy();
  });

  it("Esc 关闭设置页", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(screen.getByRole("radiogroup", { name: "权限模式" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("radiogroup", { name: "权限模式" })).toBeNull();
  });

  it("设置页打开时 Ctrl+F 不会在被遮住的对话区打开查找条", async () => {
    const { App } = await freshApp();
    render(<App />);

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });

    // 查找条若在设置页背后挂载并抢走焦点，用户会把内容输进看不见的输入框。
    expect(screen.queryByPlaceholderText("在对话中查找")).toBeNull();
  });
});

describe("侧栏就地重命名（端到端）", () => {
  it("双击侧栏标题在行内编辑，不会跑到顶栏抢焦点", async () => {
    const { App } = await freshApp();
    render(<App />);

    const title = document.querySelector(".task .t") as HTMLElement;
    expect(title).toBeTruthy();
    fireEvent.doubleClick(title);

    // 输入框必须出现在被双击的那一行……
    expect(document.querySelector(".task input.inline-input")).toBeTruthy();
    // ……而不是顶栏（两处同时挂载会让焦点飞到另一个位置）。
    expect(document.querySelector(".taskbar input.inline-input")).toBeNull();
  });
});

describe("Composer 输入体验（端到端）", () => {
  it("发送后 ↑↓ 翻输入历史，翻过头回到草稿", async () => {
    const { store, App } = await freshApp();
    const { container } = render(<App />);
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;

    fireEvent.change(ta, { target: { value: "第一条" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(store.inputHistory()).toEqual(["第一条"]));
    await waitFor(() => expect(store.getState().busy).toBe(false));

    fireEvent.change(ta, { target: { value: "第二条" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(store.inputHistory()).toEqual(["第一条", "第二条"]));

    // ↑ 翻到最近一条，再 ↑ 更早；↓ 往回翻，翻过头回到草稿（发完是空的）。
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("第二条");
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("第一条");
    fireEvent.keyDown(ta, { key: "ArrowDown" });
    expect(ta.value).toBe("第二条");
    fireEvent.keyDown(ta, { key: "ArrowDown" });
    expect(ta.value).toBe("");
  });

  it("多行输入里 ↑↓ 不接管，正常移动光标", async () => {
    const { store, App } = await freshApp();
    store.pushHistory("历史条目");
    const { container } = render(<App />);
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;

    fireEvent.change(ta, { target: { value: "第一行\n第二行" } });
    ta.setSelectionRange(ta.value.length, ta.value.length);
    fireEvent.keyDown(ta, { key: "ArrowUp" });

    // 光标不在首行：↑ 保持默认行为，输入内容不变、也不翻历史。
    expect(ta.value).toBe("第一行\n第二行");
  });

  it("@ 唤起文件选择器，选中后把路径插回输入框", async () => {
    const { App } = await freshApp();
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      if (args[0] === "list_workspace_files") return ["src/a.rs", "src/b.ts"];
      return undefined;
    });
    const { container } = render(<App />);
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;

    fireEvent.keyDown(ta, { key: "@" });
    await waitFor(() =>
      expect(screen.getByPlaceholderText("搜索工作区文件…")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: "src/a.rs" }));

    // @ 和选中的路径一起插回原光标处，选完选择器收起。
    expect(ta.value).toBe("@src/a.rs ");
    expect(screen.queryByPlaceholderText("搜索工作区文件…")).toBeNull();
  });

  it("词中间的 @ 不唤起选择器（如邮箱地址）", async () => {
    const { App } = await freshApp();
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      if (args[0] === "list_workspace_files") return ["src/a.rs"];
      return undefined;
    });
    const { container } = render(<App />);
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;

    fireEvent.change(ta, { target: { value: "联系 user" } });
    ta.setSelectionRange(ta.value.length, ta.value.length);
    fireEvent.keyDown(ta, { key: "@" });

    expect(screen.queryByPlaceholderText("搜索工作区文件…")).toBeNull();
  });
});

describe("词级高亮与变更跳转（端到端）", () => {
  const DIFF = [
    "diff --git a/src/main.rs b/src/main.rs",
    "--- a/src/main.rs",
    "+++ b/src/main.rs",
    "@@ -1,3 +1,3 @@",
    " fn main() {",
    "-    let x = 1;",
    "+    let x = 2;",
    " }",
    "@@ -8,3 +8,3 @@ fn other() {",
    "-    run(false);",
    "+    run(true);",
    " }",
  ].join("\n");

  /** 打开预览并切到「变更」视图，等 diff 渲染完。 */
  async function openDiff(text: string) {
    const { store } = await freshApp();
    const { ResultsPanel } = await import("./components/ResultsPanel");
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      const cmd = args[0];
      if (cmd === "read_file_preview") return "fn main() {}";
      if (cmd === "file_diff") return { repo: true, text };
      return undefined;
    });
    const { container } = render(<ResultsPanel hidden={false} onCollapse={() => {}} />);
    await store.openPreview("C:/ws/src/main.rs");
    fireEvent.click(screen.getByRole("button", { name: "变更" }));
    await waitFor(() => expect(container.querySelector(".dl.d-add")).not.toBeNull());
    return container;
  }

  it("同行内只有变化的词标 .wd，公共部分照常着色", async () => {
    const container = await openDiff(DIFF);

    // 删除行的「1」与新增行的「2」各自包在 .wd 里，其余文字不带词级底色。
    expect(container.querySelector(".d-del .wd")?.textContent).toBe("1");
    expect(container.querySelector(".d-add .wd")?.textContent).toBe("2");
    // 第二个变更块里的 false → true 同样精确到词。
    const wds = Array.from(container.querySelectorAll(".wd")).map((w) => w.textContent);
    expect(wds).toEqual(["1", "2", "false", "true"]);
    // 公共部分的关键字照常走语法着色，没有被 .wd 吞掉。
    expect(container.querySelector(".d-add .hljs-keyword")).not.toBeNull();
  });

  it("上一处 / 下一处变更跳转：计数、当前块高亮、越界回绕", async () => {
    const container = await openDiff(DIFF);
    const dcnt = () => container.querySelector(".dcnt")?.textContent?.replace(/\s+/g, " ").trim();
    const next = screen.getByRole("button", { name: "下一处变更" });
    const prev = screen.getByRole("button", { name: "上一处变更" });

    expect(dcnt()).toBe("0 / 2");
    expect(container.querySelector(".dl.cur")).toBeNull();

    fireEvent.click(next);
    expect(dcnt()).toBe("1 / 2");
    const rows = Array.from(container.querySelectorAll(".dl.cur"));
    expect(rows.length).toBeGreaterThan(0);
    expect(rows[0].getAttribute("data-ci")).toBe("0");
    expect(rows[0].textContent).toContain("let x");

    fireEvent.click(next);
    expect(dcnt()).toBe("2 / 2");
    expect(container.querySelector(".dl.cur")?.textContent).toContain("run");

    // 再下一处回绕到第一处；上一处从第一处同样回绕到最后一处。
    fireEvent.click(next);
    expect(dcnt()).toBe("1 / 2");
    fireEvent.click(prev);
    expect(dcnt()).toBe("2 / 2");
    fireEvent.click(prev);
    expect(dcnt()).toBe("1 / 2");
  });

  it("只有一处变更时跳转按钮禁用", async () => {
    const one = [
      "diff --git a/a.rs b/a.rs",
      "--- a/a.rs",
      "+++ b/a.rs",
      "@@ -1 +1 @@",
      "-let x = 1;",
      "+let x = 2;",
    ].join("\n");
    const container = await openDiff(one);

    expect(container.querySelector(".dcnt")?.textContent?.replace(/\s+/g, " ").trim()).toBe("0 / 1");
    expect(screen.getByRole("button", { name: "下一处变更" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "上一处变更" })).toBeDisabled();
  });
});

describe("流式输出增量渲染（端到端）", () => {
  const DELTA = "nanohermes://delta";

  /**
   * 起一轮挂起的发送：run_task 不返回，由测试控制何时收尾；
   * 返回后端增量回调与容器，测试里手动喂流式文本。
   */
  async function startStream() {
    const cbs: Record<string, (p: string) => void> = {};
    listenMock.mockImplementation(async (...args: unknown[]) => {
      cbs[String(args[0])] = args[1] as (p: string) => void;
      return () => {};
    });
    let finish!: () => void;
    invokeMock.mockImplementation(async (...args: unknown[]) => {
      if (args[0] === "run_task") {
        return new Promise<string>((r) => {
          finish = () => r("");
        });
      }
      return undefined;
    });
    const { App, store } = await freshApp();
    const { container } = render(<App />);
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "讲讲这段代码" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(cbs[DELTA]).toBeTruthy());
    return { container, store, delta: (t: string) => cbs[DELTA](t), finish: () => finish() };
  }

  it("流式期间全局状态零提交：其余组件不会被每帧唤醒（界面卡住的根治点）", async () => {
    const { container, store, delta } = await startStream();
    let commits = 0;
    const off = store.subscribe(() => {
      commits++;
    });

    delta("第一段内容。\n\n");
    delta("第二段还在写");
    await waitFor(() =>
      expect(container.querySelector(".content.stream")).toHaveTextContent("第二段还在写"),
    );

    // 实时文本已经进了气泡……
    expect(container.querySelectorAll(".content.stream")).toHaveLength(1);
    // ……但全局 store 一次都没提交：没有整树重渲染、没有每帧高度回测与强制重排，
    // 每帧只有这一个气泡自己在重绘。
    expect(commits).toBe(0);

    off();
  });

  it("稳定块只渲染一次、DOM 不重建，每帧只重绘尾块", async () => {
    const { container, delta } = await startStream();

    delta("第一段内容。\n\n");
    await waitFor(() =>
      expect(container.querySelector(".content.stream")).toHaveTextContent("第一段内容。"),
    );
    const first = container.querySelector(".content.stream > div > div") as HTMLElement;
    expect(first).not.toBeNull();
    expect(first.textContent).toContain("第一段内容。");

    // 后续增量到达：已完成的块原封不动（节点同一实例），只有尾块在长。
    delta("第二段还在写");
    await waitFor(() =>
      expect(container.querySelector(".content.stream")).toHaveTextContent("第二段还在写"),
    );
    expect(container.querySelector(".content.stream > div > div")).toBe(first);
    expect(first.isConnected).toBe(true);

    delta("，继续补充。\n\n第三段开头");
    await waitFor(() =>
      expect(container.querySelector(".content.stream")).toHaveTextContent("第三段开头"),
    );
    expect(container.querySelector(".content.stream > div > div")).toBe(first);
    // 已完成的块数从 1 变成 2，且都是各自只渲染一次。
    expect(container.querySelectorAll(".content.stream > div > div")).toHaveLength(2);
  });

  it("代码围栏内只更新文本节点，收尾转终态并补语法高亮", async () => {
    const { container, delta, finish } = await startStream();

    delta("看这段：\n\n```rust\nfn main() {}\n");
    await waitFor(() => expect(container.querySelector(".content.stream .code code")).not.toBeNull());
    const code = container.querySelector(".content.stream .code code") as HTMLElement;
    expect(code.textContent).toContain("fn main");

    // 围栏内的增量走纯文本快速路：外壳节点不变，只更新文本。
    delta("    println!(\"hi\");\n");
    await waitFor(() => expect(code.textContent).toContain("println"));
    expect(container.querySelector(".content.stream .code code")).toBe(code);
    expect(container.querySelector(".content.stream .code-head")?.textContent).toContain("rust");

    delta("```\n");
    finish();
    await waitFor(() => expect(container.querySelector(".content.stream")).toBeNull());

    // 终态整体渲染：语法高亮补齐（逐块分帧，稍后到位）、光标消失、复制等装饰齐全。
    await waitFor(() => expect(container.querySelector(".content .hljs-keyword")).not.toBeNull());
    expect(container.querySelector(".cursor")).toBeNull();
    expect(container.querySelector(".content .code-copy")).not.toBeNull();
  });

  it("大段无空行输出也逐块落盘，尾块长度有上限（否则每帧整体重解析会卡死）", async () => {
    const { container, delta } = await startStream();

    // 5 万字符、全程没有空行：这正是长段落/列表类回复的形态。
    const line = "这是一段没有空行的很长的输出内容一二三四五六七八九十。";
    const chunk = Array.from({ length: 500 }, () => line).join("\n") + "\n";
    delta(chunk);
    delta(chunk);
    delta(chunk);
    delta(chunk);
    await waitFor(() =>
      expect(container.querySelector(".content.stream")).toHaveTextContent("这是一段没有空行"),
    );

    // 已落成稳定块的部分占绝大多数，尾块只剩最后一小截参与每帧重绘。
    expect(container.querySelectorAll(".content.stream > div > div").length).toBeGreaterThan(5);
    const tail = container.querySelector(".content.stream .cursor") as HTMLElement;
    expect(tail).not.toBeNull();
    expect(tail.textContent!.length).toBeLessThan(4200);
  });

  it("长回答可以折叠成两行摘要，点击摘要块展开", async () => {
    const { container, delta, finish } = await startStream();

    delta("这是一段很长的回答内容。".repeat(40));
    finish();
    await waitFor(() => expect(container.querySelector(".content.stream")).toBeNull());

    const foldBtn = Array.from(container.querySelectorAll(".acts button")).find((b) =>
      b.textContent?.includes("折叠"),
    ) as HTMLButtonElement;
    expect(foldBtn).toBeTruthy();
    fireEvent.click(foldBtn);

    const box = container.querySelector(".foldbox") as HTMLButtonElement;
    expect(box).not.toBeNull();
    expect(box.textContent).toContain("已折叠");

    // 摘要块本身也是展开入口，不用再去找操作区按钮。
    fireEvent.click(box);
    await waitFor(() => expect(container.querySelector(".foldbox")).toBeNull());
    expect(container.querySelector(".content")).not.toBeNull();
  });

  it("点击停止后进入等待态，本轮收尾时复位并提示已停止", async () => {
    const { container, store, finish } = await startStream();
    const btn = container.querySelector(".send.stop") as HTMLButtonElement;
    expect(btn).not.toBeNull();
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    await waitFor(() => expect(store.getState().stopping).toBe(true));
    // 按钮：转圈 + 禁用，标题说明在等当前步骤结束。
    expect(btn.disabled).toBe(true);
    expect(btn.querySelector(".spin")).not.toBeNull();
    expect(btn.getAttribute("title")).toContain("等待当前步骤结束");
    // 顶栏状态从「执行中」变成「停止中」，请求已生效这件事全程可见。
    expect(container.querySelector(".state-chip")?.textContent).toBe("停止中");

    // 等待期间按 Enter 不会静默丢弃：消息进队列，输入框清空并显示队列条。
    const ta = container.querySelector("#prompt") as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "再问一句" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(store.getState().toast).toContain("已加入队列"));
    expect(ta.value).toBe("");
    await waitFor(() => expect(container.querySelector(".queuebar")).not.toBeNull());
    expect(container.querySelector(".queuebar")?.textContent).toContain("再问一句");

    // 本轮结束：等待标记复位，停止按钮换回发送键，并给一次「已停止」回执。
    finish();
    await waitFor(() => expect(store.getState().busy).toBe(false));
    expect(store.getState().stopping).toBe(false);
    await waitFor(() => expect(store.getState().toast).toBe("任务已停止"));
    await waitFor(() => expect(container.querySelector(".send.stop")).toBeNull());
    // 停止按钮消失后焦点回到输入框，键盘用户可以直接接着输入。
    await waitFor(() => expect(document.activeElement).toBe(ta));
  });
});

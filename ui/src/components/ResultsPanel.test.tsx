/**
 * 结果区预览的组件测试：代码视图的行号数量与着色是否真的落到 DOM 上。
 *
 * 与其它组件测试一致，这里把 store 换成极简替身，只喂一个已经就绪的预览状态。
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

const m = vi.hoisted(() => ({
  state: {
    filter: "all",
    panelTab: "artifacts",
    preview: {
      path: "src/main.rs",
      text: "fn main() {\n    let x = 1;\n}\n",
      loading: false,
      error: null,
    },
  },
  task: { id: "t1", location: "local", msgs: [], files: [], workspace: "", model: "" },
}));

vi.mock("../lib/store", () => ({
  useAppState: () => m.state,
  activeTask: () => m.task,
  closePreview: vi.fn(),
  openPreview: vi.fn(),
  setFilter: vi.fn(),
  setPanelTab: vi.fn(),
  addRef: vi.fn(),
  openFolderPick: vi.fn(),
  taskIsCloud: () => false,
  toast: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  inTauri: false,
  appWindow: () => null,
  invoke: vi.fn(async () => ({ repo: false, text: "" })),
  listen: vi.fn(async () => () => {}),
  errText: (e: unknown) => String(e),
}));

import { ResultsPanel } from "./ResultsPanel";

afterEach(cleanup);

describe("结果区预览", () => {
  it("代码视图按行渲染行号，并带上 highlight.js 的 token 类名", () => {
    const { container } = render(<ResultsPanel hidden={false} onCollapse={() => {}} />);

    // 文本是 3 行，末尾换行不算行号
    expect(container.querySelectorAll(".pv-gutter span")).toHaveLength(3);
    expect(container.querySelector(".pv-scroll pre")?.innerHTML).toContain("hljs-");
  });

  it("认不出的扩展名退化成纯文本，仍然有行号", () => {
    m.state.preview = { path: "notes.xyz", text: "纯文本\n第二行", loading: false, error: null };
    const { container } = render(<ResultsPanel hidden={false} onCollapse={() => {}} />);

    expect(container.querySelectorAll(".pv-gutter span")).toHaveLength(2);
    expect(container.querySelector(".pv-scroll pre")?.innerHTML).not.toContain("hljs-");
  });
});

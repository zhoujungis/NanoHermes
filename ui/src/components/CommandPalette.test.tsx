/**
 * 命令面板组件测试：初始列表、搜索过滤、空结果提示、键盘导航与条件隐藏项。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { CommandPalette } from "./CommandPalette";

const m = vi.hoisted(() => {
  const task = {
    id: "t1",
    location: "local",
    msgs: [] as unknown[],
    workspace: "",
    model: "",
  };
  const state = {
    busy: false,
    mode: "default",
    settings: null as null | {
      models: Array<{ id: string; name: string; model: string }>;
      active_model: string;
      personas?: Array<{ id: string; name: string; prompt: string }>;
      active_persona?: string;
      snippets?: Array<{ id: string; name: string; text: string }>;
    },
  };
  return {
    task,
    state,
    useAppState: () => state,
    activeTask: () => task,
    taskIsCloud: (t: { location?: string } | null) => (t?.location || "local") === "cloud",
    /** 与真实实现同义的简化版：认不出的 id 回落到默认档案，再没有就 null。 */
    resolveModel: (
      s: { models: Array<{ id: string }>; active_model: string } | null,
      id?: string | null,
    ) =>
      s?.models.find((x) => x.id === (id || s.active_model)) ??
      s?.models.find((x) => x.id === s.active_model) ??
      s?.models[0] ??
      null,
    /** 与 resolveModel 同构的简化版人设解析。 */
    resolvePersona: (
      s: { personas?: Array<{ id: string }>; active_persona?: string } | null,
      id?: string | null,
    ) => {
      const list = s?.personas || [];
      return (
        list.find((x) => x.id === (id || s?.active_persona)) ??
        list.find((x) => x.id === s?.active_persona) ??
        list[0] ??
        null
      );
    },
    closePalette: vi.fn(),
    createTask: vi.fn(async () => {}),
    exportCurrent: vi.fn(async () => {}),
    insertSnippet: vi.fn(),
    openFolderPick: vi.fn(),
    resetCurrent: vi.fn(async () => {}),
    setMode: vi.fn(async () => {}),
    setPanelTab: vi.fn(),
    setTaskLocation: vi.fn(async () => {}),
    setTaskModel: vi.fn(async () => {}),
    setTaskPersona: vi.fn(async () => {}),
    stop: vi.fn(async () => {}),
    togglePin: vi.fn(),
  };
});

vi.mock("../lib/store", () => ({
  useAppState: m.useAppState,
  activeTask: m.activeTask,
  taskIsCloud: m.taskIsCloud,
  resolveModel: m.resolveModel,
  resolvePersona: m.resolvePersona,
  closePalette: m.closePalette,
  createTask: m.createTask,
  exportCurrent: m.exportCurrent,
  insertSnippet: m.insertSnippet,
  openFolderPick: m.openFolderPick,
  resetCurrent: m.resetCurrent,
  setMode: m.setMode,
  setPanelTab: m.setPanelTab,
  setTaskLocation: m.setTaskLocation,
  setTaskModel: m.setTaskModel,
  setTaskPersona: m.setTaskPersona,
  stop: m.stop,
  togglePin: m.togglePin,
}));

const noop = () => {};

function renderPalette(onOpenShortcuts = noop) {
  return render(
    <CommandPalette
      onOpenSettings={noop}
      onOpenShortcuts={onOpenShortcuts}
      onToggleSidebar={noop}
      onToggleAside={noop}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  m.task.location = "local";
  m.task.msgs = [];
  m.task.model = "";
  m.state.busy = false;
  m.state.mode = "default";
  m.state.settings = null;
});

afterEach(async () => {
  cleanup();
  const { setTheme } = await import("../lib/theme");
  setTheme("graphite");
});

describe("初始列表", () => {
  it("列出核心命令", () => {
    renderPalette();
    expect(screen.getByText("新建本机任务")).toBeInTheDocument();
    expect(screen.getByText("新建云端任务")).toBeInTheDocument();
    expect(screen.getByText("打开设置")).toBeInTheDocument();
  });

  it("隐藏当前主题项，保留其他主题", () => {
    renderPalette();
    expect(screen.queryByText("主题：石墨")).toBeNull();
    expect(screen.getByText("主题：午夜")).toBeInTheDocument();
    expect(screen.getByText("主题：跟随系统")).toBeInTheDocument();
  });

  it("有打开快捷键面板的入口", () => {
    const onOpenShortcuts = vi.fn();
    renderPalette(onOpenShortcuts);
    fireEvent.click(screen.getByText("键盘快捷键"));
    expect(onOpenShortcuts).toHaveBeenCalled();
  });
});

describe("搜索", () => {
  it("按关键字过滤", () => {
    renderPalette();
    const input = screen.getByPlaceholderText("搜索命令…");

    fireEvent.change(input, { target: { value: "设置" } });

    expect(screen.getByText("打开设置")).toBeInTheDocument();
    expect(screen.queryByText("新建本机任务")).toBeNull();
  });

  it("没有匹配时给出空状态", () => {
    renderPalette();
    fireEvent.change(screen.getByPlaceholderText("搜索命令…"), {
      target: { value: "zzzz" },
    });
    expect(screen.getByText("没有匹配的命令")).toBeInTheDocument();
  });
});

describe("键盘", () => {
  it("回车执行当前选中项（首项为新建本机任务）", () => {
    renderPalette();
    const input = screen.getByPlaceholderText("搜索命令…");

    fireEvent.keyDown(input, { key: "Enter" });

    expect(m.closePalette).toHaveBeenCalled();
    expect(m.createTask).toHaveBeenCalledWith(false, "local");
  });

  it("Esc 关闭面板", () => {
    renderPalette();
    fireEvent.keyDown(screen.getByPlaceholderText("搜索命令…"), { key: "Escape" });
    expect(m.closePalette).toHaveBeenCalled();
  });
});

describe("条件隐藏", () => {
  it("云端任务不出现停止项与模式分组", () => {
    m.task.location = "cloud";
    m.state.busy = true;
    renderPalette();

    expect(screen.queryByText("停止正在执行的任务")).toBeNull();
    expect(screen.queryByText("切换到 Plan 模式")).toBeNull();
  });

  it("本地任务运行时可停止", () => {
    m.state.busy = true;
    renderPalette();
    expect(screen.getByText("停止正在执行的任务")).toBeInTheDocument();
  });

  it("没有对话时不出现导出项", () => {
    renderPalette();
    expect(screen.queryByText("导出对话为 Markdown")).toBeNull();
  });

  it("有对话时出现导出项", () => {
    m.task.msgs = [{ k: "user" }];
    renderPalette();
    expect(screen.getByText("导出对话为 Markdown")).toBeInTheDocument();
  });
});

describe("模型", () => {
  const withModels = () => {
    m.state.settings = {
      models: [
        { id: "a", name: "甲", model: "ma" },
        { id: "b", name: "乙", model: "mb" },
      ],
      active_model: "a",
    };
  };

  it("列出模型项，且隐藏当前在用的那个", () => {
    withModels();
    renderPalette();

    // 任务没指定模型 → 用默认的 a，所以 a 隐藏、b 可选。
    expect(screen.queryByText("使用模型：甲")).toBeNull();
    expect(screen.getByText("使用模型：乙")).toBeInTheDocument();
  });

  it("切换模型项会带上任务与档案 id", () => {
    withModels();
    renderPalette();

    fireEvent.click(screen.getByText("使用模型：乙"));

    expect(m.setTaskModel).toHaveBeenCalledWith("t1", "b");
  });

  it("云端任务不列模型项", () => {
    m.task.location = "cloud";
    withModels();
    renderPalette();
    expect(screen.queryByText(/使用模型：/)).toBeNull();
  });
});

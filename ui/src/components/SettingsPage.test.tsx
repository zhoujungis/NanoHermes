/**
 * 设置页组件测试：主题切换与设置项交互。
 *
 * store 用假实现替换（这些用例只关心设置页自己的行为），但主题模块用真的 ——
 * 主题切换的副作用（<html data-theme>）正需要被真实验证。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { SettingsPage } from "./SettingsPage";
import type { Settings } from "../lib/store";

const m = vi.hoisted(() => {
  const state = {
    settings: null as unknown as Settings | null,
    mode: "default",
    settingsSync: "idle" as string,
    settingsSyncError: "",
  };
  return {
    state,
    autosaveSettings: vi.fn(),
    flushSettings: vi.fn(async () => true),
    setMode: vi.fn(async () => {}),
    settleSettings: vi.fn(),
  };
});

vi.mock("../lib/store", () => ({
  useAppState: () => m.state,
  autosaveSettings: m.autosaveSettings,
  flushSettings: m.flushSettings,
  setMode: m.setMode,
  settleSettings: m.settleSettings,
}));

/** 两份现成的模型档案，够用来验证增删改与「设为默认」。 */
const settings: Settings = {
  api_key: "",
  base_url: "",
  model: "",
  models: [
    { id: "a", name: "甲", base_url: "https://a/v1", api_key: "ka", model: "ma" },
    { id: "b", name: "乙", base_url: "https://b/v1", api_key: "kb", model: "mb" },
  ],
  active_model: "a",
  personas: [{ id: "default", name: "默认", prompt: "" }],
  active_persona: "default",
  snippets: [],
  workspace: "C:/ws",
  mode: "default",
  location: "local",
  cloud_url: "",
  cloud_token: "",
  thinking: false,
  project_docs: true,
  memory: true,
  temperature: 0,
  max_tokens: 4096,
};

beforeEach(() => {
  vi.clearAllMocks();
  m.state.mode = "default";
  m.state.settingsSync = "idle";
  m.state.settings = { ...settings, models: settings.models.map((x) => ({ ...x })) };
});

afterEach(async () => {
  // 先卸载再复位主题：否则通知订阅者会在 act 之外触发组件重渲染。
  cleanup();
  const { setTheme } = await import("../lib/theme");
  setTheme("graphite");
  delete document.documentElement.dataset.theme;
});

describe("主题", () => {
  it("渲染时高亮当前主题", () => {
    render(<SettingsPage onClose={() => {}} />);
    expect(screen.getByRole("radio", { name: "石墨" })).toHaveClass("on");
  });

  it("点击素笺会写入 <html data-theme> 并高亮该项", () => {
    render(<SettingsPage onClose={() => {}} />);

    const paper = screen.getByRole("radio", { name: "素笺" });
    expect(paper).not.toHaveClass("on");

    fireEvent.click(paper);

    expect(document.documentElement.dataset.theme).toBe("paper");
    expect(screen.getByRole("radio", { name: "素笺" })).toHaveClass("on");
  });
});

describe("键盘遍历", () => {
  it("主题组是单选组，选中项可 Tab 进入、其余跳过", () => {
    render(<SettingsPage onClose={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "界面主题" });
    const items = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));
    // roving tabindex：只有选中项（当前主题）在 Tab 序里。
    expect(items.filter((el) => el.tabIndex === 0)).toHaveLength(1);
    expect(items.find((el) => el.tabIndex === 0)).toHaveAttribute("aria-checked", "true");
  });

  it("方向键在主题组内移动并选中", () => {
    render(<SettingsPage onClose={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "界面主题" });
    const items = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));
    items[0].focus();

    fireEvent.keyDown(group, { key: "ArrowRight" });
    expect(items[1]).toHaveClass("on");
    expect(items[1]).toHaveFocus();

    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(items[0]).toHaveClass("on");

    // 到头循环：第一项再往左就跳到最后一项。
    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(items[items.length - 1]).toHaveClass("on");
  });

  it("权限模式组用 Home/End 跳首尾并切换模式", () => {
    render(<SettingsPage onClose={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "权限模式" });

    fireEvent.keyDown(group, { key: "End" });
    expect(m.setMode).toHaveBeenLastCalledWith("debug");

    fireEvent.keyDown(group, { key: "Home" });
    expect(m.setMode).toHaveBeenLastCalledWith("default");
  });

  it("执行位置组方向键切换并写出 location", () => {
    render(<SettingsPage onClose={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "默认执行位置" });
    const items = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));
    items[0].focus();

    fireEvent.keyDown(group, { key: "ArrowDown" });

    expect(items[1]).toHaveFocus();
    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.location).toBe("cloud");
  });

  it("高级开关暴露按下状态", () => {
    render(<SettingsPage onClose={() => {}} />);
    expect(screen.getByTitle("深度思考开关")).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByTitle("长期记忆开关")).toHaveAttribute("aria-pressed", "true");
  });
});

describe("设置项", () => {
  it("列表里已有两套模型，且默认项被标出", () => {
    render(<SettingsPage onClose={() => {}} />);

    expect(screen.getByDisplayValue("甲")).toBeInTheDocument();
    expect(screen.getByDisplayValue("乙")).toBeInTheDocument();
    // 默认项是甲：它的那颗按钮显示为「默认项」，乙的是「设为默认」。
    expect(screen.getByRole("button", { name: /默认项/ })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /设为默认/ })).toHaveLength(1);
  });

  it("修改档案里的模型名会即时保存整份列表", () => {
    render(<SettingsPage onClose={() => {}} />);

    // 两张卡各有一个「模型」输入框：改第一张（甲）的。
    fireEvent.change(screen.getAllByPlaceholderText("gpt-4o-mini")[0], {
      target: { value: "my-model" },
    });

    expect(m.autosaveSettings).toHaveBeenCalled();
    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.models[0].model).toBe("my-model");
    // 改一张卡不能动到另一张
    expect(last.models[1].model).toBe("mb");
  });

  it("把乙设为默认会同时写出 active_model", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: /设为默认/ }));

    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.active_model).toBe("b");
    expect(screen.getByRole("button", { name: /默认项/ })).toBeInTheDocument();
  });

  it("添加模型会往列表里补一张空卡", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: /添加模型/ }));

    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.models).toHaveLength(3);
    expect(last.models[2].name).toBe("新模型");
  });

  it("点击权限模式按钮会切换模式", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByRole("radio", { name: /^Plan/ }));

    expect(m.setMode).toHaveBeenCalledWith("plan");
  });

  it("顶栏保存按钮触发立即落盘", () => {
    render(<SettingsPage onClose={() => {}} />);
    fireEvent.click(screen.getByTitle("立即保存"));
    expect(m.flushSettings).toHaveBeenCalled();
  });

  it("关闭按钮回调 onClose", () => {
    const onClose = vi.fn();
    render(<SettingsPage onClose={onClose} />);
    fireEvent.click(screen.getByTitle("关闭（Esc）"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("人设、片段与高级参数", () => {
  it("添加人设会往列表追加一条并写出整份 settings", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: /添加人设/ }));

    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.personas).toHaveLength(2);
    expect(last.personas[1].name).toBe("新的人设");
  });

  it("编辑人设的附加指令会即时保存，且去掉首尾空白", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText("留空表示不附加任何指令"), {
      target: { value: "  用中文回答  " },
    });

    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.personas[0].prompt).toBe("用中文回答");
  });

  it("添加片段并写入内容", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: /添加片段/ }));
    const first = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(first.snippets).toHaveLength(1);
    expect(first.snippets[0].name).toBe("新片段");

    fireEvent.change(screen.getByPlaceholderText("插入到输入框的文本"), {
      target: { value: "总结一下这个仓库" },
    });

    const last = m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings;
    expect(last.snippets[0].text).toBe("总结一下这个仓库");
  });

  it("高级开关分别写出 thinking 与 project_docs", () => {
    render(<SettingsPage onClose={() => {}} />);

    fireEvent.click(screen.getByTitle("深度思考开关"));
    expect((m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings).thinking).toBe(true);

    fireEvent.click(screen.getByTitle("读取项目指南开关"));
    expect((m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings).project_docs).toBe(false);
  });

  it("长期记忆开关写出 memory 且默认开启", () => {
    render(<SettingsPage onClose={() => {}} />);

    // 默认开启：按钮显示「已开启」。
    expect(screen.getByTitle("长期记忆开关").textContent).toBe("已开启");

    fireEvent.click(screen.getByTitle("长期记忆开关"));
    expect((m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings).memory).toBe(false);
  });

  it("数值参数会被夹到合法区间后保存", () => {
    render(<SettingsPage onClose={() => {}} />);

    // 两个数字输入依次是：温度、最大输出 tokens。
    const nums = screen.getAllByRole("spinbutton");
    fireEvent.change(nums[1], { target: { value: "999999" } });
    expect((m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings).max_tokens).toBe(200000);

    fireEvent.change(nums[0], { target: { value: "5" } });
    expect((m.autosaveSettings.mock.calls.at(-1)?.[0] as Settings).temperature).toBe(2);
  });

  it("清空数字输入框不会把最小值写进配置", () => {
    render(<SettingsPage onClose={() => {}} />);
    const nums = screen.getAllByRole("spinbutton");
    const before = m.autosaveSettings.mock.calls.length;

    // 清空后应保持空白，让用户重新输入；以前 Number("") === 0 会立刻夹成 1。
    fireEvent.change(nums[1], { target: { value: "" } });
    expect((nums[1] as HTMLInputElement).value).toBe("");
    expect(m.autosaveSettings.mock.calls.length).toBe(before);

    // 失焦时还原成上一次生效的值，并兜底落盘。
    fireEvent.blur(nums[1]);
    expect((nums[1] as HTMLInputElement).value).toBe("4096");
    expect(m.flushSettings).toHaveBeenCalled();
  });
});

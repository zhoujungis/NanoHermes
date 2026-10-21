/**
 * 主题模块的纯逻辑测试。
 *
 * theme.ts 里 `current` 是模块级状态，所以每个用例都重新 import 一份全新的模块，
 * 才能真实还原「启动时读 localStorage」这件事。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { setThemeMock } = vi.hoisted(() => ({ setThemeMock: vi.fn(async () => {}) }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: setThemeMock }),
}));

const LS_KEY = "nanohermes.theme.v1";
const DARK = "(prefers-color-scheme: dark)";

type Listener = (e: { matches: boolean }) => void;

/** 装一个可控的系统明暗探针，并返回一个手动翻转系统设置的方法。 */
function installMatchMedia(initialDark: boolean) {
  const listeners = new Set<Listener>();
  let dark = initialDark;
  const mql = {
    get matches() {
      return dark;
    },
    media: DARK,
    onchange: null,
    addEventListener: (_t: string, l: Listener) => listeners.add(l),
    removeEventListener: (_t: string, l: Listener) => listeners.delete(l),
    addListener: (l: Listener) => listeners.add(l),
    removeListener: (l: Listener) => listeners.delete(l),
    dispatchEvent: () => true,
  };
  window.matchMedia = (() => mql) as unknown as typeof window.matchMedia;
  return {
    setDark(v: boolean) {
      dark = v;
      listeners.forEach((l) => l({ matches: v }));
    },
  };
}

async function fresh() {
  vi.resetModules();
  return await import("./theme");
}

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  setThemeMock.mockClear();
});

afterEach(() => {
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("resolveTheme", () => {
  it("非 auto 的主题原样返回", async () => {
    const { resolveTheme } = await fresh();
    expect(resolveTheme("graphite")).toBe("graphite");
    expect(resolveTheme("midnight")).toBe("midnight");
    expect(resolveTheme("paper")).toBe("paper");
  });

  it("auto 在系统深色时落到石墨", async () => {
    const { resolveTheme } = await fresh();
    installMatchMedia(true);
    expect(resolveTheme("auto")).toBe("graphite");
  });

  it("auto 在系统浅色时落到素笺", async () => {
    const { resolveTheme } = await fresh();
    installMatchMedia(false);
    expect(resolveTheme("auto")).toBe("paper");
  });
});

describe("恢复上次选择", () => {
  it("没有存储内容时默认石墨", async () => {
    const { getTheme } = await fresh();
    expect(getTheme()).toBe("graphite");
  });

  it("从 localStorage 恢复上次选择", async () => {
    localStorage.setItem(LS_KEY, "paper");
    const { getTheme } = await fresh();
    expect(getTheme()).toBe("paper");
  });

  it("存储内容非法时回退到石墨", async () => {
    localStorage.setItem(LS_KEY, "neon");
    const { getTheme } = await fresh();
    expect(getTheme()).toBe("graphite");
  });
});

describe("setTheme", () => {
  it("写入存储、更新 <html data-theme> 并通知订阅者", async () => {
    const { setTheme, getTheme, subscribe } = await fresh();
    const spy = vi.fn();
    const off = subscribe(spy);

    setTheme("midnight");

    expect(getTheme()).toBe("midnight");
    expect(localStorage.getItem(LS_KEY)).toBe("midnight");
    expect(document.documentElement.dataset.theme).toBe("midnight");
    expect(spy).toHaveBeenCalledTimes(1);
    off();
  });

  it("设置成当前主题不产生任何副作用", async () => {
    const { setTheme, subscribe } = await fresh();
    const spy = vi.fn();
    subscribe(spy);

    setTheme("graphite"); // 当前已经是 graphite

    expect(spy).not.toHaveBeenCalled();
    expect(localStorage.getItem(LS_KEY)).toBeNull();
  });

  it("非法主题值被忽略", async () => {
    const { setTheme, getTheme } = await fresh();
    setTheme("neon" as never);
    expect(getTheme()).toBe("graphite");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("在 Tauri 内把明暗同步给原生窗口", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const { setTheme } = await fresh();

    setTheme("paper");
    expect(setThemeMock).toHaveBeenLastCalledWith("light");

    setTheme("midnight");
    expect(setThemeMock).toHaveBeenLastCalledWith("dark");
  });
});

describe("initTheme", () => {
  it("auto 模式下随系统明暗实时切换", async () => {
    localStorage.setItem(LS_KEY, "auto");
    const mm = installMatchMedia(false);
    const { initTheme, getTheme } = await fresh();

    expect(getTheme()).toBe("auto");
    initTheme();
    expect(document.documentElement.dataset.theme).toBe("paper");

    mm.setDark(true);
    expect(document.documentElement.dataset.theme).toBe("graphite");
  });

  it("固定主题不受系统明暗影响", async () => {
    localStorage.setItem(LS_KEY, "midnight");
    const mm = installMatchMedia(false);
    const { initTheme } = await fresh();

    initTheme();
    expect(document.documentElement.dataset.theme).toBe("midnight");

    mm.setDark(true);
    expect(document.documentElement.dataset.theme).toBe("midnight");
  });
});

describe("选项元数据", () => {
  it("THEME_META 覆盖全部主题且每个都带名称与色卡", async () => {
    const { THEME_META, THEMES } = await fresh();
    expect(THEME_META.map((t) => t.id)).toEqual(THEMES);
    for (const t of THEME_META) {
      expect(t.name.length).toBeGreaterThan(0);
      expect(t.swatch.length).toBeGreaterThan(0);
    }
  });
});

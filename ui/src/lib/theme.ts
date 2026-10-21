/**
 * 主题：把「用哪套配色」集中在一个小模块里。
 *
 * 配色本身写在 styles/tokens.css 的三套 [data-theme] 块里（石墨 / 午夜 / 素笺），
 * 这里只负责挑一套、写到 <html data-theme> 上、记住用户的选择，并让「跟随系统」
 * 随系统明暗实时切换。
 */
import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type Theme = "auto" | "graphite" | "midnight" | "paper";
export type Resolved = "graphite" | "midnight" | "paper";

export const THEMES: Theme[] = ["auto", "graphite", "midnight", "paper"];

/** 供设置页与命令面板共用的选项元数据；swatch 是该主题的代表色，用于预览。 */
export const THEME_META: Array<{ id: Theme; name: string; hint: string; swatch: string[] }> = [
  { id: "auto", name: "跟随系统", hint: "随 Windows 浅色/深色设置切换", swatch: ["#0d1117", "#f5f6f8"] },
  { id: "graphite", name: "石墨", hint: "中性深灰，默认", swatch: ["#0b0c0f", "#23272e", "#8b8cf0"] },
  { id: "midnight", name: "午夜", hint: "冷调深蓝", swatch: ["#090c14", "#1f2637", "#6ea8fe"] },
  { id: "paper", name: "素笺", hint: "干净浅色", swatch: ["#ffffff", "#eceef2", "#5457d6"] },
];

const LS_KEY = "nanohermes.theme.v1";
const DARK_QUERY = "(prefers-color-scheme: dark)";

const isTheme = (v: unknown): v is Theme => THEMES.includes(v as Theme);

/** 「跟随系统」时按系统明暗落到具体主题，其余原样返回。 */
export function resolveTheme(t: Theme): Resolved {
  if (t !== "auto") return t;
  if (typeof window === "undefined" || !window.matchMedia) return "graphite";
  return window.matchMedia(DARK_QUERY).matches ? "graphite" : "paper";
}

function read(): Theme {
  try {
    const raw = localStorage.getItem(LS_KEY);
    return isTheme(raw) ? raw : "graphite";
  } catch {
    return "graphite";
  }
}

let current: Theme = read();
const listeners = new Set<() => void>();

function apply(t: Theme): void {
  if (typeof document === "undefined") return;
  const resolved = resolveTheme(t);
  document.documentElement.dataset.theme = resolved;
  // 原生窗口的边框/阴影跟随主题；权限缺失时静默跳过，不影响界面。
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    try {
      void getCurrentWindow()
        .setTheme(resolved === "paper" ? "light" : "dark")
        .catch(() => {});
    } catch {
      /* 非 Tauri 环境 */
    }
  }
}

export function getTheme(): Theme {
  return current;
}

export function setTheme(t: Theme): void {
  if (!isTheme(t) || t === current) return;
  current = t;
  try {
    localStorage.setItem(LS_KEY, t);
  } catch {
    /* 隐私模式/配额不足：仍切换本会话外观 */
  }
  apply(t);
  listeners.forEach((l) => l());
}

export function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

/** 订阅当前主题；切换后组件自动重渲染。 */
export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, getTheme, getTheme);
}

/** 启动时调用一次：恢复上次选择，并让「跟随系统」跟随系统明暗变化。 */
export function initTheme(): void {
  apply(current);
  window.matchMedia?.(DARK_QUERY).addEventListener?.("change", () => {
    if (current === "auto") apply(current);
  });
}

/**
 * Tauri 桥接层：所有需要桌面运行时的调用都收敛在这里。
 *
 * 浏览器里直接打开 Vite dev server 调试时（例如做纯 UI 检查），`invoke` 会立刻
 * 返回一个可读的错误而不是抛出不透明的异常，`listen` 退化成空实现，界面仍然
 * 可以正常渲染。
 */
import { invoke as coreInvoke } from "@tauri-apps/api/core";
import { listen as coreListen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 是否运行在 Tauri 运行时中。 */
export const inTauri =
  typeof (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
  "undefined";

export function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!inTauri) {
    return Promise.reject(new Error("当前不在 NanoHermes 桌面环境中运行"));
  }
  return coreInvoke<T>(cmd, args);
}

export async function listen<T>(
  event: string,
  cb: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return coreListen<T>(event, (e) => cb(e.payload));
}

/** 主窗口；非桌面环境返回 null，调用方自行降级。 */
export function appWindow() {
  if (!inTauri) return null;
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

/** 统一把各种 reject 值转成可读文案。 */
export function errText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

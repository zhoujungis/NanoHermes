/**
 * 窗口几何状态的持久化：记住上次的尺寸与位置，下次启动时恢复。
 * 只在桌面运行时生效；浏览器里调试时静默跳过，不影响界面渲染。
 */
import { PhysicalPosition, PhysicalSize, availableMonitors } from "@tauri-apps/api/window";
import { appWindow, inTauri } from "./tauri";

const KEY = "nanohermes.window.v1";
const MIN_W = 900;
const MIN_H = 600;

interface Geo {
  x: number;
  y: number;
  w: number;
  h: number;
}

function read(): Geo | null {
  try {
    const g = JSON.parse(localStorage.getItem(KEY) || "null");
    if (!g || typeof g.w !== "number" || typeof g.h !== "number") return null;
    return g as Geo;
  } catch {
    return null;
  }
}

/**
 * 恢复上次的窗口尺寸与位置。
 * 尺寸按当前显示器上限夹取；位置若已被显示器裁掉大半则丢弃，避免开在屏幕外。
 */
export async function restoreWindowState(): Promise<void> {
  if (!inTauri) return;
  const win = appWindow();
  const g = read();
  if (!win || !g) return;
  try {
    const mons = await availableMonitors();
    const maxW = Math.max(MIN_W, ...mons.map((m) => m.size.width));
    const maxH = Math.max(MIN_H, ...mons.map((m) => m.size.height));
    const w = Math.min(Math.max(g.w, MIN_W), maxW);
    const h = Math.min(Math.max(g.h, MIN_H), maxH);
    await win.setSize(new PhysicalSize(w, h));

    const visible = mons.some(
      (m) =>
        g.x >= m.position.x - 40 &&
        g.y >= m.position.y - 40 &&
        g.x + 140 <= m.position.x + m.size.width &&
        g.y + 60 <= m.position.y + m.size.height,
    );
    if (visible) await win.setPosition(new PhysicalPosition(g.x, g.y));
  } catch {
    /* 恢复失败时保持默认窗口，不阻断启动 */
  }
}

/** 监听尺寸/位置变化并落盘（去抖，避免拖动过程中高频写入）。返回取消函数。 */
export async function trackWindowState(): Promise<() => void> {
  if (!inTauri) return () => {};
  const win = appWindow();
  if (!win) return () => {};

  let timer = 0;
  const save = async () => {
    try {
      // 最大化时不覆盖「还原后」的尺寸，否则下次还原会开成全屏大小。
      if (await win.isMaximized()) return;
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      localStorage.setItem(
        KEY,
        JSON.stringify({ x: pos.x, y: pos.y, w: size.width, h: size.height }),
      );
    } catch {
      /* 写入失败不影响使用 */
    }
  };
  const bump = () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void save(), 400);
  };

  const un1 = await win.onResized(bump);
  const un2 = await win.onMoved(bump);
  return () => {
    window.clearTimeout(timer);
    un1();
    un2();
  };
}

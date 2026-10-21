/**
 * 测试环境公共装配：在跑每个测试文件前执行一次。
 *
 * 补上 jsdom 没有实现的浏览器 API，缺了它们相关组件在测试里会直接抛错：
 * 1) jest-dom 断言扩展（toBeInTheDocument 之类）；
 * 2) window.matchMedia —— 主题的「跟随系统」依赖它；
 * 3) Element.scrollIntoView —— 命令面板用它把选中项滚进可视区。
 */
import "@testing-library/jest-dom/vitest";

if (typeof window !== "undefined" && !window.matchMedia) {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();

  const query = (q: string): MediaQueryList => {
    const mql = {
      matches: false,
      media: q,
      onchange: null,
      addEventListener: (_t: string, l: (e: MediaQueryListEvent) => void) => {
        listeners.add(l);
      },
      removeEventListener: (_t: string, l: (e: MediaQueryListEvent) => void) => {
        listeners.delete(l);
      },
      addListener: (l: (e: MediaQueryListEvent) => void) => {
        listeners.add(l);
      },
      removeListener: (l: (e: MediaQueryListEvent) => void) => {
        listeners.delete(l);
      },
      dispatchEvent: () => true,
    };
    return mql as unknown as MediaQueryList;
  };

  window.matchMedia = query as typeof window.matchMedia;
}

if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

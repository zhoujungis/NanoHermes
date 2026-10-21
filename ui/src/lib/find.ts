/**
 * 会话内查找（Ctrl+F）。
 *
 * 分成两层：纯函数只负责在消息文本里数出所有命中，DOM 层负责把命中定位成
 * Range 并交给浏览器的高亮接口。分开的好处是计数逻辑可以直接跑单测，而定位
 * 必须依赖真实 DOM（Markdown、代码块、复制按钮都会被排除掉）。
 */
import type { Msg } from "./store";

/** 一处命中：`msg` 是消息下标，`rank` 是它在这条消息内第几次出现（从 1 起）。 */
export interface Hit {
  msg: number;
  rank: number;
}

/** 只有这三种消息带正文，工具行不参与查找。 */
export function findableText(m: Msg): string {
  return m.k === "user" || m.k === "ai" || m.k === "note" ? m.t || "" : "";
}

/** 找出全部命中（大小写不敏感）；查询为空或全是空白时不算命中。 */
export function findHits(msgs: Msg[], query: string): Hit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const hits: Hit[] = [];
  msgs.forEach((m, i) => {
    const text = findableText(m).toLowerCase();
    if (!text) return;
    let from = 0;
    let rank = 0;
    for (;;) {
      const at = text.indexOf(q, from);
      if (at < 0) break;
      hits.push({ msg: i, rank: ++rank });
      from = at + q.length;
    }
  });
  return hits;
}

/** 装饰性文本（代码块语言标签、复制按钮、消息下方的操作按钮）不参与定位。 */
const SKIP_SELECTOR = ".code-head, .acts, .uacts";

function textNodes(el: Element): Text[] {
  const out: Text[] = [];
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = (node as Text).parentElement;
      if (!parent || parent.closest(SKIP_SELECTOR)) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let node = walker.nextNode();
  while (node) {
    out.push(node as Text);
    node = walker.nextNode();
  }
  return out;
}

/** 把整段文本里的字符区间映射回具体的文本节点位置。 */
function pointAt(nodes: Text[], pos: number): { node: Text; offset: number } {
  let acc = 0;
  for (const node of nodes) {
    const len = node.data.length;
    if (pos <= acc + len) return { node, offset: pos - acc };
    acc += len;
  }
  const last = nodes[nodes.length - 1];
  return { node: last, offset: last.data.length };
}

/**
 * 在某个消息元素里定位第 `rank` 个命中，返回可用来高亮/滚动的 Range。
 * 跨文本节点的命中（例如加粗词被拆成多段）也能正确覆盖。
 */
export function locateRange(el: Element, needle: string, rank: number): Range | null {
  const q = needle.trim().toLowerCase();
  if (!q) return null;
  const nodes = textNodes(el);
  if (!nodes.length) return null;
  const text = nodes.map((n) => n.data).join("");
  const lower = text.toLowerCase();

  let from = 0;
  let at = -1;
  for (let found = 1; ; found++) {
    const i = lower.indexOf(q, from);
    if (i < 0) return null;
    if (found === rank) {
      at = i;
      break;
    }
    from = i + q.length;
  }

  const start = pointAt(nodes, at);
  const end = pointAt(nodes, at + q.length);
  const range = document.createRange();
  try {
    range.setStart(start.node, start.offset);
    range.setEnd(end.node, end.offset);
  } catch {
    return null;
  }
  return range;
}

/**
 * 按命中列表在消息区里收集 Range。消息元素用 `data-mi` 标注下标，
 * 找不到（例如消息已被截断）就跳过，不抛错。
 */
export function collectRanges(root: ParentNode, hits: Hit[], needle: string): Range[] {
  const out: Range[] = [];
  hits.forEach((h) => {
    const el = root.querySelector(`[data-mi="${h.msg}"]`);
    const range = el && locateRange(el, needle, h.rank);
    if (range) out.push(range);
  });
  return out;
}

type HighlightCtor = new (...ranges: Range[]) => unknown;

/** 浏览器是否支持 CSS Custom Highlight（Chromium 105+ / WebView2 均支持）。 */
export function supportsHighlight(): boolean {
  const api = (CSS as unknown as { highlights?: Map<string, unknown> } | undefined)?.highlights;
  return !!api && typeof (globalThis as { Highlight?: HighlightCtor }).Highlight === "function";
}

/** 把命中交给浏览器高亮：`nh-find` 是所有命中，`nh-find-cur` 是当前这一处。 */
export function paintHighlight(ranges: Range[], current: number): void {
  if (!supportsHighlight()) return;
  const hl = (globalThis as unknown as { Highlight: HighlightCtor }).Highlight;
  const api = (CSS as unknown as { highlights: Map<string, unknown> }).highlights;
  api.set("nh-find", new hl(...ranges));
  if (ranges[current]) api.set("nh-find-cur", new hl(ranges[current]));
  else api.delete("nh-find-cur");
}

export function clearHighlight(): void {
  const api = (CSS as unknown as { highlights?: Map<string, unknown> } | undefined)?.highlights;
  if (!api) return;
  api.delete("nh-find");
  api.delete("nh-find-cur");
}

/**
 * 把命中滚到可视区（落在上方三分之一处，能带出后文）。
 * 返回命中的矩形；拿不到布局时返回 null，调用方可以改走选中兜底。
 */
export function scrollRangeIntoView(scroller: HTMLElement, range: Range): DOMRect | null {
  const rect = range.getBoundingClientRect();
  const box = scroller.getBoundingClientRect();
  if (!rect.height && !rect.width) return null;
  const delta = rect.top - box.top - box.height / 3;
  if (Math.abs(delta) > 8) scroller.scrollTo({ top: scroller.scrollTop + delta, behavior: "smooth" });
  return rect;
}

/** 不支持高亮接口时的兜底：用选区把当前命中标出来。 */
export function selectRange(range: Range): void {
  const sel = window.getSelection?.();
  if (!sel) return;
  try {
    sel.removeAllRanges();
    sel.addRange(range);
  } catch {
    /* 极端情况下选区不可用：忽略，滚动定位仍然有效 */
  }
}

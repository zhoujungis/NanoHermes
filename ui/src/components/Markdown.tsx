import { useEffect, useMemo, useRef } from "react";
import {
  codeShellHtml,
  enhance,
  escapeHtml,
  highlightBlocks,
  renderMarkdown,
  splitMarkdown,
} from "../lib/markdown";
import { useStreamText } from "../lib/store";

/**
 * 一条消息的内容区：Markdown 渲染 + 净化 + 代码块装饰。
 *
 * 流式输出走「增量渲染」：整条消息按围栏外的空行切成块，已完成的块只渲染
 * 一次并保留 DOM，每帧真正重绘的只有仍在增长的尾块。此前每 40ms 对整条消息
 * 重跑一遍「marked 解析 → DOMParser 净化 → innerHTML 整体重建 → 代码块重新
 * 包壳」，开销随消息总长线性膨胀，长回复会把主线程彻底占死（界面和鼠标都卡住）。
 *
 * 尾块在未闭合代码围栏里时连 Markdown 都不解析，只更新 <code> 的文本节点。
 * 终态（streaming=false）整体渲染一次并补上语法高亮。
 *
 * 流式期间的实时文本走独立订阅通道（useStreamText），不进 React 全局状态：
 * 只有「正在输出的这一条」订阅，每帧只重渲染它自己，侧栏 / 结果区 / 顶栏 /
 * 输入区完全不动——这是「输出时整个界面卡住」的根治点。
 */
export function Markdown({
  text,
  streaming,
  err,
}: {
  text: string;
  streaming?: boolean;
  err?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);

  // 只有真正的流式气泡订阅通道；终态与非流式直接用 props 里的 text。
  const live = useStreamText(!!streaming && !err);
  const src = streaming && !err ? live : text;

  // 流式增量渲染的状态；src 记住已累计的原文，内容被整体替换时重建。
  const grow = useRef<{
    n: number;
    src: string;
    stable: HTMLElement;
    tail: HTMLElement;
    mode: "" | "md" | "code";
  } | null>(null);

  // 流式期间不做全量解析（那是卡顿的来源），终态才整体渲染一次。
  const html = useMemo(() => {
    if (streaming) return "";
    return err ? `<p>${escapeHtml(src)}</p>` : renderMarkdown(src);
  }, [src, err, streaming]);
  // 对象身份要稳定：React 按引用比较 dangerouslySetInnerHTML，每次重渲染都换
  // 一个新对象会把 enhance 加的代码块外壳整个重置掉。
  const htmlProp = useMemo(
    () => (streaming ? undefined : { __html: html }),
    [html, streaming],
  );

  // 流式：稳定块只追加一次，尾块每帧只重绘这一小段。
  useEffect(() => {
    const node = ref.current;
    if (!node || err || !streaming) return;
    let g = grow.current;
    if (!g || g.stable.parentNode !== node || !src.startsWith(g.src)) {
      node.textContent = "";
      const stable = document.createElement("div");
      const tail = document.createElement("div");
      tail.className = "cursor";
      node.append(stable, tail);
      g = { n: 0, src: "", stable, tail, mode: "" };
      grow.current = g;
    }
    g.src = src;

    const { done, tail, inFence } = splitMarkdown(src);
    for (let i = g.n; i < done.length; i++) {
      const div = document.createElement("div");
      div.innerHTML = renderMarkdown(done[i]);
      enhance(div, false);
      g.stable.appendChild(div);
    }
    g.n = done.length;

    const el = g.tail;
    if (inFence) {
      // 围栏里只有纯代码：不解析 Markdown，只更新文本节点，连 innerHTML 重建都省掉。
      const info = tail.match(/^\s*(`{3,}|~{3,})([^\n]*)/) || [];
      const body = tail.replace(/^\s*(`{3,}|~{3,})[^\n]*\n?/, "");
      if (g.mode !== "code") {
        el.innerHTML = codeShellHtml((info[2] || "").trim());
        g.mode = "code";
      }
      const code = el.querySelector("code");
      if (code && code.textContent !== body) code.textContent = body;
    } else {
      if (g.mode !== "md") {
        el.textContent = "";
        el.className = "cursor";
        g.mode = "md";
      }
      if (tail.trim()) {
        el.innerHTML = renderMarkdown(tail);
        enhance(el, false);
      } else {
        el.textContent = "";
      }
    }
  }, [src, streaming, err]);

  // 终态：整体渲染一次，清掉流式的输入光标；语法高亮逐块分帧补上——
  // 大回复里所有代码块一次性高亮是长任务，会在收尾瞬间把界面按住（鼠标都卡）。
  useEffect(() => {
    const node = ref.current;
    if (!node || err || streaming || !src) return;
    node.querySelectorAll(".cursor").forEach((n) => n.classList.remove("cursor"));
    enhance(node, false);
    highlightBlocks(node);
  }, [html, streaming, err, src]);

  if (!src && !err) {
    return (
      <div className={"content" + (streaming ? " stream" : "")}>
        <div className="dots">
          <i />
          <i />
          <i />
        </div>
      </div>
    );
  }

  return (
    <div
      ref={ref}
      className={"content" + (err ? " err" : "") + (streaming ? " stream" : "")}
      // 流式期间由 effect 增量维护 DOM，React 不碰它的子节点。
      dangerouslySetInnerHTML={htmlProp}
    />
  );
}

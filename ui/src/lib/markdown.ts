/**
 * Markdown 渲染与净化。
 *
 * 模型输出属于不可信内容，因此渲染管线是「先净化、后装饰」：
 *   1. marked 解析成 HTML；
 *   2. sanitize 按白名单剥离标签/属性（只保留排版所需的标签）；
 *   3. enhance 在净化后的 DOM 上追加代码块外壳、高亮、表格横向滚动。
 * 第 3 步必须在 DOM 上做，才能放行 <button>（复制按钮）而不用把 button 写进白名单。
 */
import { marked } from "marked";
import { hljs } from "./hl";

marked.use({ gfm: true, breaks: true });

export function escapeHtml(s: unknown): string {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

const OK_TAGS = new Set([
  "P", "BR", "STRONG", "EM", "DEL", "CODE", "PRE", "BLOCKQUOTE", "UL", "OL", "LI",
  "H1", "H2", "H3", "H4", "H5", "H6", "A", "HR", "TABLE", "THEAD", "TBODY", "TR",
  "TH", "TD", "SPAN", "INPUT", "SUP", "SUB", "DIV",
]);

const OK_ATTR: Record<string, string[]> = {
  A: ["href", "title"],
  CODE: ["class"],
  PRE: ["class"],
  SPAN: ["class"],
  INPUT: ["type", "checked", "disabled"],
  OL: ["start"],
  LI: ["class"],
  DIV: ["class"],
  TABLE: ["class"],
  TH: ["align"],
  TD: ["align"],
};

// Markdown 渲染器自己产生的类名。模型输出可以写任意 class，若原样放行，
// 一句 `<div class="lightbox">` 就能盖住整个界面（这些类在 shell.css 里是
// position:fixed + 全屏遮罩）。
const OK_CLASS = /^(language-[A-Za-z0-9_+-]+|hljs|hljs-[A-Za-z0-9_-]+|task-list-item|contains-task-list)$/;

function filterClasses(el: HTMLElement): void {
  const raw = el.getAttribute("class") || "";
  const kept = raw.split(/\s+/).filter((c) => c && OK_CLASS.test(c));
  if (kept.length) el.setAttribute("class", kept.join(" "));
  else el.removeAttribute("class");
}

export function sanitize(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.body.querySelectorAll<HTMLElement>("*").forEach((el) => {
    const tag = el.tagName;
    if (!OK_TAGS.has(tag)) {
      // 未知标签：保留文字内容，丢掉标签本身。
      const parent = el.parentNode;
      if (!parent) return;
      while (el.firstChild) parent.insertBefore(el.firstChild, el);
      parent.removeChild(el);
      return;
    }
    const allowed = OK_ATTR[tag] || [];
    Array.from(el.attributes).forEach((a) => {
      if (!allowed.includes(a.name.toLowerCase())) el.removeAttribute(a.name);
    });
    if (allowed.includes("class")) filterClasses(el);
    if (tag === "A") {
      const href = el.getAttribute("href") || "";
      if (!/^https?:\/\//i.test(href)) el.removeAttribute("href");
      else {
        el.setAttribute("target", "_blank");
        el.setAttribute("rel", "noreferrer noopener");
      }
    }
    if (tag === "INPUT") {
      if (el.getAttribute("type") !== "checkbox") {
        el.remove();
        return;
      }
      el.setAttribute("disabled", "");
    }
  });
  return doc.body.innerHTML;
}

export function renderMarkdown(src: string): string {
  const text = src || "";
  const html = marked.parse(text, { async: false });
  return sanitize(typeof html === "string" ? html : "");
}

/** 单个流式块的大小上限（字符）：超过就按行硬切，见 splitMarkdown。 */
const CHUNK_MAX = 4000;

/**
 * 把流式中的 Markdown 切成稳定块与尾块。
 *
 * 增量渲染的基础：输出是追加式的，块的边界一旦定下就不会再变，可以只渲染一次；
 * 每帧真正重新解析的只剩尾块。收块边界有三条规则（都只依赖前缀，追加不会让已定
 * 的边界挪动）：
 *   1. 围栏外的空行；
 *   2. 代码围栏的闭合行——整块代码从此不会再变，立即收块；
 *   3. 围栏外的块累计超过 CHUNK_MAX 就按行硬切——真实输出常常大段没有空行
 *      （长段落、列表），尾块若不设上限就会无限膨胀，每帧都要整体重解析，
 *      长回复照样会把界面卡死。围栏内不硬切，避免把一个代码块劈成两半。
 */
export function splitMarkdown(src: string): {
  /** 已完成、内容不会再变的块。 */
  done: string[];
  /** 尾块：仍在增长中，每帧重绘。 */
  tail: string;
  /** 尾块是否位于未闭合的代码围栏里（此时它只是纯代码文本）。 */
  inFence: boolean;
} {
  const lines = (src || "").split("\n");
  const done: string[] = [];
  let fence = false;
  let start = 0;
  let size = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const isFenceLine = /^\s*(`{3,}|~{3,})/.test(line);
    if (isFenceLine) fence = !fence;
    if (!fence && line.trim() === "") {
      if (i > start) done.push(lines.slice(start, i).join("\n"));
      start = i + 1;
      size = 0;
      continue;
    }
    size += line.length + 1;
    if (isFenceLine && !fence) {
      // 围栏闭合：这是一块完整的代码，直接落成稳定块。
      done.push(lines.slice(start, i + 1).join("\n"));
      start = i + 1;
      size = 0;
      continue;
    }
    if (!fence && size >= CHUNK_MAX) {
      done.push(lines.slice(start, i + 1).join("\n"));
      start = i + 1;
      size = 0;
    }
  }
  return { done, tail: lines.slice(start).join("\n"), inFence: fence };
}

/**
 * 流式代码块的外壳：与 `enhance` 注入的结构一致，正文随后增量填入。
 * 围栏内的尾块走这条路——不解析 Markdown、不重建 innerHTML，只更新文本节点。
 */
export function codeShellHtml(lang: string): string {
  return (
    `<div class="code"><div class="code-head">` +
    `<span>${escapeHtml(lang || "text")}</span>` +
    `<button class="${CODE_COPY_CLASS}" type="button">复制</button>` +
    `</div><pre><code></code></pre></div>`
  );
}

/** 复制到剪贴板，并在按钮上给出短暂反馈。 */
export function copyText(text: string, btn?: HTMLElement | null): void {
  const flash = () => {
    if (!btn) return;
    const old = btn.textContent;
    btn.textContent = "已复制";
    window.setTimeout(() => {
      btn.textContent = old;
    }, 1200);
  };
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text).then(flash).catch(() => {});
    return;
  }
  const ta = document.createElement("textarea");
  ta.value = text;
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
    flash();
  } catch {
    /* 复制失败时静默，不打断阅读 */
  }
  document.body.removeChild(ta);
}

const CODE_COPY_CLASS = "code-copy";

/**
 * 装饰已渲染的 Markdown DOM：代码块加外壳与复制按钮、表格加横向滚动容器。
 *
 * `final` 为 false（流式进行中）时跳过 highlight.js —— 每一帧都重新分词会明显
 * 拖慢渲染；流式结束后用 `highlightBlocks` 逐块补一次终态高亮。
 */
export function enhance(root: HTMLElement, final: boolean): void {
  root.querySelectorAll("pre").forEach((pre) => {
    if (pre.parentElement?.classList.contains("code")) return;
    const code = pre.querySelector("code");
    const cls = code?.className || "";
    const lang = (cls.match(/language-([\w+#.-]+)/) || [])[1] || "";

    const wrap = document.createElement("div");
    wrap.className = "code";
    const head = document.createElement("div");
    head.className = "code-head";
    head.innerHTML = `<span>${escapeHtml(lang || "text")}</span>`;
    const btn = document.createElement("button");
    btn.className = CODE_COPY_CLASS;
    btn.type = "button";
    btn.textContent = "复制";
    head.appendChild(btn);

    pre.parentNode?.insertBefore(wrap, pre);
    wrap.appendChild(head);
    wrap.appendChild(pre);

    if (final && code && !code.hasAttribute("data-highlighted")) {
      try {
        hljs.highlightElement(code);
      } catch {
        /* 未知语言或异常输入：保持纯文本 */
      }
    }
  });

  root.querySelectorAll("table").forEach((t) => {
    if (t.parentElement?.classList.contains("table-wrap")) return;
    const wrap = document.createElement("div");
    wrap.className = "table-wrap";
    t.parentNode?.insertBefore(wrap, t);
    wrap.appendChild(t);
  });
}

/**
 * 逐块补终态语法高亮：每个代码块一个宏任务。
 *
 * 大回复里所有代码块一次性高亮是个长任务，会在收尾瞬间把界面按住（鼠标都卡）；
 * 拆成逐块分帧后主线程始终留有空闲。已高亮或已被卸载的块直接跳过。
 */
export function highlightBlocks(root: HTMLElement): void {
  const blocks = Array.from(root.querySelectorAll<HTMLElement>("pre code"));
  let i = 0;
  const step = () => {
    while (i < blocks.length) {
      const code = blocks[i++];
      if (!code.isConnected || code.hasAttribute("data-highlighted")) continue;
      try {
        hljs.highlightElement(code);
      } catch {
        /* 未知语言或异常输入：保持纯文本 */
      }
      break;
    }
    if (i < blocks.length) window.setTimeout(step, 0);
  };
  if (blocks.length) window.setTimeout(step, 0);
}

/** 事件委托：处理 enhance 注入的复制按钮。 */
export function handleCopyClick(target: EventTarget | null): boolean {
  const el = target instanceof HTMLElement ? target.closest(`.${CODE_COPY_CLASS}`) : null;
  if (!el) return false;
  const code = el.closest(".code")?.querySelector("code");
  copyText(code?.textContent || "", el as HTMLElement);
  return true;
}

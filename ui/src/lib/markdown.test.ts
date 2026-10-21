/**
 * Markdown 渲染与净化的测试。
 *
 * 重点在「不可信内容的安全性」：模型输出会直接进 innerHTML，因此 script、事件
 * 属性、javascript: 链接都必须在这里被剥掉。
 */
import { describe, expect, it } from "vitest";
import {
  codeShellHtml,
  enhance,
  escapeHtml,
  handleCopyClick,
  renderMarkdown,
  sanitize,
  splitMarkdown,
} from "./markdown";

describe("splitMarkdown", () => {
  it("按围栏外的空行切块，尾块是仍在增长的最后一段", () => {
    const r = splitMarkdown("第一段。\n\n第二段。\n\n第三段开头");
    expect(r.done).toEqual(["第一段。", "第二段。"]);
    expect(r.tail).toBe("第三段开头");
    expect(r.inFence).toBe(false);
  });

  it("代码围栏里的空行不切，围栏闭合整块落成稳定块", () => {
    const r = splitMarkdown("```rust\nfn a() {}\n\nfn b() {}\n```");
    expect(r.done).toEqual(["```rust\nfn a() {}\n\nfn b() {}\n```"]);
    expect(r.tail).toBe("");
    expect(r.inFence).toBe(false); // 围栏已闭合

    const open = splitMarkdown("```rust\nfn a() {}\n\nfn b() {}\n");
    expect(open.done).toEqual([]);
    expect(open.inFence).toBe(true);
  });

  it("围栏闭合后紧跟文字（没有空行）也不会把整块代码留在尾块里", () => {
    const r = splitMarkdown("```js\ncode\n```\n紧跟着的文字");
    expect(r.done).toEqual(["```js\ncode\n```"]);
    expect(r.tail).toBe("紧跟着的文字");
    expect(r.inFence).toBe(false);
  });

  it("大段没有空行的输出按大小硬切，尾块长度有上限", () => {
    // 2000 行、全程没有空行：旧规则下整段都是尾块，每帧整体重解析。
    const line = "这是一段没有空行的很长的输出内容一二三四五六七八九十。";
    const src = Array.from({ length: 2000 }, () => line).join("\n");
    const r = splitMarkdown(src);
    expect(r.done.length).toBeGreaterThan(5);
    expect(r.tail.length).toBeLessThan(line.length + 4100);
    // 所有块拼回去仍是原文（切块不丢内容）。
    expect([...r.done, r.tail].join("\n")).toBe(src);
    expect(r.inFence).toBe(false);
  });

  it("围栏闭合后的空行恢复切块", () => {
    const r = splitMarkdown("```js\ncode\n```\n\n后面一段。\n\n再来");
    expect(r.done).toEqual(["```js\ncode\n```", "后面一段。"]);
    expect(r.tail).toBe("再来");
  });

  it("连续空行不产生空块，空输入得到空结果", () => {
    expect(splitMarkdown("a\n\n\n\nb")).toEqual({ done: ["a"], tail: "b", inFence: false });
    expect(splitMarkdown("")).toEqual({ done: [], tail: "", inFence: false });
    expect(splitMarkdown("没有空行的单块")).toEqual({
      done: [],
      tail: "没有空行的单块",
      inFence: false,
    });
  });
});

describe("codeShellHtml", () => {
  it("外壳与 enhance 注入的结构一致，正文可增量填入", () => {
    const html = codeShellHtml("rust");
    expect(html).toContain('<div class="code">');
    expect(html).toContain('<div class="code-head">');
    expect(html).toContain(">rust<");
    expect(html).toContain('class="code-copy"');
    expect(html).toMatch(/<pre><code><\/code><\/pre>/);
  });

  it("语言名经过转义，缺省显示 text", () => {
    expect(codeShellHtml('<img src=x onerror="1">')).toContain("&lt;img");
    expect(codeShellHtml("")).toContain(">text<");
  });
});

describe("escapeHtml", () => {
  it("转义全部 HTML 敏感字符", () => {
    expect(escapeHtml(`<a href="x">&'`)).toBe("&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
  });

  it("null/undefined 变成空串", () => {
    expect(escapeHtml(null)).toBe("");
    expect(escapeHtml(undefined)).toBe("");
  });
});

describe("sanitize", () => {
  it("未知标签被剥掉但保留其文字", () => {
    expect(sanitize("<foo>hi</foo>")).toBe("hi");
    expect(sanitize('<script>alert(1)</script>')).not.toContain("<script");
  });

  it("白名单外的属性被移除", () => {
    expect(sanitize('<p onclick="x()" style="color:red">t</p>')).toBe("<p>t</p>");
  });

  it("http(s) 链接保留并补上 target/rel", () => {
    const html = sanitize('<a href="https://example.com">x</a>');
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer noopener"');
  });

  it("javascript: 之类的危险链接被摘掉", () => {
    expect(sanitize('<a href="javascript:alert(1)">x</a>')).not.toContain("javascript:");
    expect(sanitize('<a href="/relative">x</a>')).not.toContain("href=");
  });

  it("复选框保留且被禁用，其他 input 直接删除", () => {
    expect(sanitize('<input type="checkbox">')).toContain("disabled");
    expect(sanitize('<input type="text">')).toBe("");
  });
});

describe("renderMarkdown", () => {
  it("渲染加粗与表格", () => {
    expect(renderMarkdown("**粗**")).toContain("<strong>粗</strong>");
    const table = renderMarkdown("| a | b |\n| - | - |\n| 1 | 2 |");
    expect(table).toContain("<table");
    expect(table).toContain("<th");
    expect(table).toContain("<td");
  });

  it("图片与脚本不会留在输出里", () => {
    const html = renderMarkdown('<img src=x onerror="alert(1)">\n<script>alert(1)</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
  });

  it("空输入返回空串", () => {
    expect(renderMarkdown("")).toBe("");
  });
});

describe("enhance", () => {
  function box(html: string): HTMLElement {
    const root = document.createElement("div");
    root.innerHTML = html;
    return root;
  }

  it("代码块套上外壳、标出语言并带复制按钮", () => {
    const root = box('<pre><code class="language-js">const a = 1;</code></pre>');
    enhance(root, false);

    expect(root.querySelector(".code")).toBeTruthy();
    expect(root.querySelector(".code-head span")?.textContent).toBe("js");
    expect(root.querySelector(".code-copy")?.textContent).toBe("复制");
  });

  it("重复调用不会重复套壳", () => {
    const root = box("<pre><code>x</code></pre>");
    enhance(root, false);
    enhance(root, false);
    expect(root.querySelectorAll(".code").length).toBe(1);
  });

  it("表格包进横向滚动容器", () => {
    const root = box("<table><tbody><tr><td>1</td></tr></tbody></table>");
    enhance(root, false);
    expect(root.querySelector(".table-wrap table")).toBeTruthy();
  });

  it("终态时才做语法高亮", () => {
    const root = box('<pre><code class="language-js">const a = 1;</code></pre>');
    enhance(root, true);
    expect(root.querySelector("code")?.getAttribute("data-highlighted")).toBe("yes");
  });
});

describe("handleCopyClick", () => {
  it("命中复制按钮时返回 true，其他目标返回 false", () => {
    const root = document.createElement("div");
    root.innerHTML = "<pre><code>x</code></pre>";
    enhance(root, false);

    expect(handleCopyClick(root.querySelector(".code-copy"))).toBe(true);
    expect(handleCopyClick(document.createElement("span"))).toBe(false);
    expect(handleCopyClick(null)).toBe(false);
  });
});

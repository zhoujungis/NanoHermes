import { describe, expect, it } from "vitest";
import {
  changeBlocks,
  diffLineHtml,
  parseDiff,
  wordDiff,
  wordRangesFor,
  type WordRange,
} from "./diff";

describe("parseDiff", () => {
  it("区分 hunk 头、文件元信息、增删行与上下文行", () => {
    const lines = parseDiff(
      [
        "diff --git a/src/a.rs b/src/a.rs",
        "index 1234..5678 100644",
        "--- a/src/a.rs",
        "+++ b/src/a.rs",
        "@@ -1,2 +1,2 @@",
        " let x = 1;",
        "-let y = 2;",
        "+let y = 3;",
      ].join("\n"),
    );
    expect(lines.map((l) => l.kind)).toEqual([
      "meta",
      "meta",
      "meta",
      "meta",
      "hunk",
      "ctx",
      "del",
      "add",
    ]);
  });

  it("行首标记与正文分离，上下文行也对齐同一列", () => {
    const lines = parseDiff("-let y = 2;\n+let y = 3;\n let x = 1;");
    expect(lines[0]).toEqual({ kind: "del", marker: "-", code: "let y = 2;" });
    expect(lines[1]).toEqual({ kind: "add", marker: "+", code: "let y = 3;" });
    expect(lines[2]).toEqual({ kind: "ctx", marker: " ", code: "let x = 1;" });
  });

  it("进了 hunk 之后，以 --- 开头的删除行不会被当成文件头", () => {
    const lines = parseDiff("@@ -1 +1 @@\n---\n+++");
    expect(lines[1].kind).toBe("del");
    expect(lines[1].code).toBe("--");
    expect(lines[2].kind).toBe("add");
    expect(lines[2].code).toBe("++");
  });

  it("没有 hunk 头时 --- / +++ 仍是元信息", () => {
    const lines = parseDiff("--- a/x\n+++ b/x");
    expect(lines.map((l) => l.kind)).toEqual(["meta", "meta"]);
  });
});

describe("diffLineHtml", () => {
  it("增删行正文按语言做词法高亮，行首标记单独成段", () => {
    const [line] = parseDiff('+fn main() { println!("hi"); }');
    const html = diffLineHtml(line, "rust");
    expect(html.startsWith('<span class="dm">+</span>')).toBe(true);
    // 关键字与字符串各归各色，而不是整行一个颜色。
    expect(html).toContain("hljs-keyword");
    expect(html).toContain("hljs-string");
  });

  it("hunk 头与元信息只转义不着色", () => {
    const [meta] = parseDiff("+++ b/a<b>.rs");
    const html = diffLineHtml(meta, "rust");
    expect(html).toContain("a&lt;b&gt;.rs");
    expect(html).not.toContain("hljs-");
  });

  it("拿不准语言时退回纯转义文本", () => {
    const [line] = parseDiff('-echo "<b>"');
    const html = diffLineHtml(line, "");
    expect(html).toContain("&lt;b&gt;");
    expect(html).not.toContain("hljs-");
  });

  it("空正文补零宽字符保住行高", () => {
    const [line] = parseDiff("+");
    expect(diffLineHtml(line, "")).toContain("&#8203;");
  });

  it("给出词级区间时，只有变化的词包 .wd，其余仍按语言着色", () => {
    const [line] = parseDiff("+let y = 3;");
    const ranges: WordRange[] = [{ start: 8, end: 9 }];
    const html = diffLineHtml(line, "rust", ranges);
    expect(html).toContain('<span class="wd">');
    // 变化词是「3」，公共部分的关键词依旧走 hljs。
    expect(html).toMatch(/<span class="wd">.*3.*<\/span>/);
    expect(html).toContain("hljs-keyword");
    // .wd 只包住变化的词，不该把整行都包进去。
    const wd = html.slice(html.indexOf('<span class="wd">'));
    expect(wd).not.toContain("let");
  });
});

describe("wordDiff", () => {
  it("只改一个词时精确标出该词的区间", () => {
    const { a, b } = wordDiff("let y = 2;", "let y = 3;");
    expect(a).toEqual([{ start: 8, end: 9 }]);
    expect(b).toEqual([{ start: 8, end: 9 }]);
    // 区间对应的原文正好是被改的字符。
    expect("let y = 2;".slice(a[0].start, a[0].end)).toBe("2");
    expect("let y = 3;".slice(b[0].start, b[0].end)).toBe("3");
  });

  it("整行重写时退化为整行变化（等同不标词级的精确版）", () => {
    const { a, b } = wordDiff("foo", "bar baz");
    expect(a).toEqual([{ start: 0, end: 3 }]);
    expect(b).toEqual([{ start: 0, end: 7 }]);
  });

  it("纯空白差异不产生词级区间", () => {
    const { a, b } = wordDiff("let x = 1;  ", "let x = 1;\n");
    expect(a).toEqual([]);
    expect(b).toEqual([]);
  });

  it("删光 / 新增内容时区间覆盖整段", () => {
    const { a, b } = wordDiff("old text", "");
    expect(a).toEqual([{ start: 0, end: 8 }]);
    expect(b).toEqual([]);
  });
});

describe("wordRangesFor", () => {
  it("删除行与新增行按序两两配对做词级 diff", () => {
    const lines = parseDiff("-let y = 2;\n-let z = 4;\n+let y = 3;\n+let z = 5;");
    const ranges = wordRangesFor(lines);
    expect(ranges).toHaveLength(4);
    expect(ranges[0]).toEqual([{ start: 8, end: 9 }]);
    expect(ranges[2]).toEqual([{ start: 8, end: 9 }]);
    expect("let z = 4;".slice(ranges[1][0].start, ranges[1][0].end)).toBe("4");
    expect("let z = 5;".slice(ranges[3][0].start, ranges[3][0].end)).toBe("5");
  });

  it("数量配不上的行不标词级，上下文行恒为空", () => {
    // 两个删除行只有一个新增行：首尾配对后第二行删除配不上，退回整行底色。
    const lines = parseDiff("-a\n-b\n+c\n ctx");
    const ranges = wordRangesFor(lines);
    expect(ranges[1]).toEqual([]); // 配不上对
    expect(ranges[3]).toEqual([]); // 上下文行
    expect(ranges[0]).toEqual([{ start: 0, end: 1 }]);
    expect(ranges[2]).toEqual([{ start: 0, end: 1 }]);
  });

  it("两行内容完全相同时不标词级", () => {
    const ranges = wordRangesFor(parseDiff("-foo();\n+foo();"));
    expect(ranges[0]).toEqual([]);
    expect(ranges[1]).toEqual([]);
  });

  it("新增在前删除在后（add 块先出现）也能配对", () => {
    const lines = parseDiff("+let y = 3;\n-let y = 2;");
    const ranges = wordRangesFor(lines);
    expect(ranges[0]).toEqual([{ start: 8, end: 9 }]);
    expect(ranges[1]).toEqual([{ start: 8, end: 9 }]);
  });
});

describe("changeBlocks", () => {
  it("连续增删行归并为一个变更块", () => {
    const lines = parseDiff([
      "@@ -1,5 +1,5 @@",
      " ctx",
      "-a",
      "-b",
      "+c",
      " ctx",
      "-d",
      " ctx",
    ].join("\n"));
    expect(changeBlocks(lines)).toEqual([
      { start: 2, end: 5 },
      { start: 6, end: 7 },
    ]);
  });

  it("纯上下文 diff 没有变更块", () => {
    expect(changeBlocks(parseDiff(" a\n b"))).toEqual([]);
  });
});

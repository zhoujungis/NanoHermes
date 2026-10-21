/**
 * 结果区代码着色的测试：只验证语言推断、行号与转义，DOM 呈现交给组件。
 */
import { describe, expect, it } from "vitest";
import { countLines, highlightCode, langOf } from "./code";

describe("langOf", () => {
  it("按扩展名识别常见语言", () => {
    expect(langOf("src/main.rs")).toBe("rust");
    expect(langOf("a/b/App.tsx")).toBe("typescript");
    expect(langOf("gui/capabilities\\default.json")).toBe("json");
    expect(langOf("README.md")).toBe("markdown");
  });

  it("认不出就不着色，不硬猜", () => {
    expect(langOf("Dockerfile")).toBe("");
    expect(langOf("data.xyz")).toBe("");
    expect(langOf("Makefile")).toBe("");
  });
});

describe("countLines", () => {
  it("普通文本按换行数计行", () => {
    expect(countLines("a\nb\nc")).toBe(3);
  });

  it("末尾换行多出的空行不占编号", () => {
    expect(countLines("a\nb\n")).toBe(2);
  });

  it("空文件也按一行显示", () => {
    expect(countLines("")).toBe(1);
  });
});

describe("highlightCode", () => {
  it("着色结果带 hljs 的 token 类名", () => {
    const html = highlightCode("fn main() { let x = 1; }", "rust");
    expect(html).toContain("hljs-");
  });

  it("没有语言时原样转义，避免把代码当标签解析", () => {
    const html = highlightCode("<script>alert(1)</script>", "");
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });

  it("文件过大时跳过着色，仍然转义输出", () => {
    const big = "<b>" + "x".repeat(200_001);
    const html = highlightCode(big, "xml");
    expect(html).toContain("&lt;b&gt;");
  });
});

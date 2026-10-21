/**
 * 会话内查找的测试：只覆盖纯逻辑与 DOM 定位（高亮接口由浏览器提供，这里不做断言）。
 */
import { describe, expect, it } from "vitest";
import { findHits, locateRange } from "./find";
import type { Msg } from "./store";

const msgs: Msg[] = [
  { k: "user", t: "NanoHermes 怎么用？nanoh 只是前缀", ts: 1 },
  { k: "tool", name: "read_file", summary: "NanoHermes/README.md", ts: 2 },
  { k: "ai", t: "读 NanoHermes 的文档", ts: 3 },
  { k: "note", t: "已切换到 NanoHermes", ts: 4 },
];

describe("findHits", () => {
  it("大小写不敏感，工具行不参与查找", () => {
    expect(findHits(msgs, "nanoh")).toEqual([
      { msg: 0, rank: 1 },
      { msg: 0, rank: 2 },
      { msg: 2, rank: 1 },
      { msg: 3, rank: 1 },
    ]);
  });

  it("空查询或全是空白时不算命中", () => {
    expect(findHits(msgs, "")).toEqual([]);
    expect(findHits(msgs, "   ")).toEqual([]);
  });

  it("没有这个词时返回空数组", () => {
    expect(findHits(msgs, "不存在的词")).toEqual([]);
  });
});

describe("locateRange", () => {
  it("定位第 n 处命中，跨文本节点也能覆盖", () => {
    const el = document.createElement("div");
    el.innerHTML = "<p>alpha <b>beta</b> alpha</p>";

    const first = locateRange(el, "alpha", 1);
    expect(first?.toString()).toBe("alpha");

    // 第二处在加粗之后：把选区截到它的起点，应当正好是「alpha beta 」
    const second = locateRange(el, "alpha", 2);
    expect(second?.toString()).toBe("alpha");
    const head = document.createRange();
    head.selectNodeContents(el);
    head.setEnd(second!.startContainer, second!.startOffset);
    expect(head.toString()).toBe("alpha beta ");
  });

  it("命中数量不够时返回 null", () => {
    const el = document.createElement("div");
    el.textContent = "只有一个 alpha";
    expect(locateRange(el, "alpha", 2)).toBeNull();
    expect(locateRange(el, "alpha", 0)).toBeNull();
  });

  it("跳过代码块头与消息操作区的装饰文字", () => {
    const el = document.createElement("div");
    el.innerHTML =
      '<div class="code-head"><span>复制</span></div><p>复制这段文字</p><div class="acts"><span>复制</span></div>';
    // 只有正文里的那一个「复制」算命中
    expect(locateRange(el, "复制", 1)?.startContainer.parentElement?.tagName).toBe("P");
    expect(locateRange(el, "复制", 2)).toBeNull();
  });
});

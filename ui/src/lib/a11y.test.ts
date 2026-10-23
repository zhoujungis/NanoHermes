import { describe, expect, it } from "vitest";
import type { KeyboardEvent } from "react";
import { rowButton } from "./a11y";

/** 只用到 key 与 preventDefault，构造一个最小的键盘事件即可。 */
function key(k: string, log: string[]): KeyboardEvent {
  return {
    key: k,
    preventDefault: () => log.push("prevent"),
  } as unknown as KeyboardEvent;
}

describe("rowButton", () => {
  it("补上按钮语义与可聚焦属性", () => {
    const props = rowButton(() => {});
    expect(props.role).toBe("button");
    expect(props.tabIndex).toBe(0);
  });

  it("回车与空格触发动作，其它按键忽略", () => {
    const log: string[] = [];
    const props = rowButton(() => log.push("go"));

    props.onKeyDown(key("Enter", log));
    props.onKeyDown(key(" ", log));
    props.onKeyDown(key("a", log));

    // 空格会滚动页面，必须先拦截再触发
    expect(log).toEqual(["prevent", "go", "prevent", "go"]);
  });

  it("行内按钮冒泡上来的 Enter 不被整行抢走", () => {
    const log: string[] = [];
    const props = rowButton(() => log.push("go"));
    const fromNested = {
      key: "Enter",
      preventDefault: () => log.push("prevent"),
      target: { tagName: "BUTTON" },
      currentTarget: { tagName: "DIV" },
    } as unknown as KeyboardEvent;

    props.onKeyDown(fromNested);
    // 既不触发切任务，也不 preventDefault（否则按钮自己不会激活）。
    expect(log).toEqual([]);
  });
});

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
});

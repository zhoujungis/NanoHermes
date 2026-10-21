/**
 * 键盘可达性小工具。
 *
 * 界面里有几处「整行可点」的容器（任务行、文件行、目录行、附件缩略图）用的是 div /
 * img 加 onClick：鼠标没问题，但 Tab 键走不到、回车也没反应。这里把补齐 role、
 * tabIndex 与 Enter/Space 三件事收到一个函数里，避免每处各写一遍。
 */
import type { KeyboardEvent } from "react";

export interface RowButtonProps {
  role: "button";
  tabIndex: number;
  onKeyDown: (e: KeyboardEvent) => void;
}

/** 给整行可点的容器补上按钮语义与键盘操作；点击逻辑仍由各自的 onClick 负责。 */
export function rowButton(onActivate: () => void): RowButtonProps {
  return {
    role: "button",
    tabIndex: 0,
    onKeyDown: (e) => {
      if (e.key !== "Enter" && e.key !== " ") return;
      // 空格默认会滚动页面，这里要拦掉。
      e.preventDefault();
      onActivate();
    },
  };
}

/**
 * 单选选项组（主题 / 权限模式 / 执行位置）的键盘遍历。
 *
 * 挂在 role="radiogroup" 的容器上：方向键在选项间移动焦点并顺手选中，Home/End
 * 跳到首尾，到头循环。选中直接 click 目标项，复用各按钮自己的 onClick，
 * 不引入第二套选中逻辑。配合 roving tabindex（选中项 tabIndex=0，其余 -1），
 * Tab 进组一次，组内全靠方向键。
 */
export function radioGroupKeys(e: KeyboardEvent<HTMLElement>): void {
  const nav = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "Home", "End"];
  if (!nav.includes(e.key)) return;
  const box = e.currentTarget;
  const items = Array.from(box.querySelectorAll<HTMLElement>('[role="radio"]:not(:disabled)'));
  if (!items.length) return;
  e.preventDefault();
  const cur = items.indexOf(document.activeElement as HTMLElement);
  const forward = e.key === "ArrowRight" || e.key === "ArrowDown";
  let next: number;
  if (e.key === "Home") next = 0;
  else if (e.key === "End") next = items.length - 1;
  else if (cur < 0) next = forward ? 0 : items.length - 1;
  else if (forward) next = (cur + 1) % items.length;
  else next = (cur - 1 + items.length) % items.length;
  items[next].focus();
  items[next].click();
}

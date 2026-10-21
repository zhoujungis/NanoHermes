/**
 * 快捷键提示面板测试：清单渲染、按键展示与三种关闭方式。
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { ShortcutsPanel } from "./ShortcutsPanel";
import { SHORTCUTS } from "../lib/shortcuts";

afterEach(cleanup);

describe("ShortcutsPanel", () => {
  it("按分组渲染全部快捷键说明", () => {
    render(<ShortcutsPanel onClose={() => {}} />);
    for (const g of SHORTCUTS) {
      expect(screen.getByText(g.group)).toBeInTheDocument();
      for (const item of g.items) expect(screen.getByText(item.desc)).toBeInTheDocument();
    }
  });

  it("组合键拆成独立按键块展示", () => {
    render(<ShortcutsPanel onClose={() => {}} />);
    const keys = screen.getAllByText("Ctrl").map((el) => el.textContent);
    expect(keys.length).toBeGreaterThan(0);
    expect(screen.getAllByText("K").length).toBeGreaterThan(0);
  });

  it("点关闭按钮触发 onClose", () => {
    const onClose = vi.fn();
    render(<ShortcutsPanel onClose={onClose} />);
    fireEvent.click(screen.getByLabelText("关闭快捷键面板"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("点遮罩触发 onClose，点面板本体不关", () => {
    const onClose = vi.fn();
    const { container } = render(<ShortcutsPanel onClose={onClose} />);
    fireEvent.mouseDown(screen.getByRole("dialog"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(container.querySelector(".cmd-back")!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Esc 关闭面板", () => {
    const onClose = vi.fn();
    render(<ShortcutsPanel onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

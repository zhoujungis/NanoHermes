import { describe, expect, it } from "vitest";
import { ESTIMATE, FULL_LIMIT, planWindow, type PlanPart } from "./virtual";

/** 取出所有渲染段拼起来的下标。 */
const slots = (parts: PlanPart[]): number[] =>
  parts.flatMap((p) => (p.kind === "slots" ? p.indices : []));

/** 取出所有占位段的总高度。 */
const gapHeight = (parts: PlanPart[]): number =>
  parts.reduce((n, p) => n + (p.kind === "gap" ? p.height : 0), 0);

/** 渲染段与占位段交替铺满 [0, count) 且下标不重不漏。 */
function assertCovers(parts: PlanPart[], count: number, heights: number[]): void {
  let at = 0;
  for (const p of parts) {
    if (p.kind === "slots") {
      expect(p.indices[0]).toBe(at);
      at += p.indices.length;
    } else {
      expect(p.from).toBe(at);
      expect(p.to).toBeGreaterThan(p.from);
      let sum = 0;
      for (let i = p.from; i < p.to; i++) sum += heights[i] > 0 ? heights[i] : ESTIMATE;
      expect(p.height).toBe(sum);
      at = p.to;
    }
  }
  expect(at).toBe(count);
}

describe("planWindow", () => {
  it("空列表没有渲染计划", () => {
    expect(planWindow(0, [], 0, 600)).toEqual([]);
  });

  it("短会话全量渲染，不产生占位", () => {
    const parts = planWindow(FULL_LIMIT, [], 0, 600);
    expect(parts).toHaveLength(1);
    expect(parts[0]).toEqual({ kind: "slots", indices: Array.from({ length: FULL_LIMIT }, (_, i) => i) });
  });

  it("长会话只渲染可视区加 overscan，前后留占位", () => {
    const count = 200;
    const parts = planWindow(count, [], 50 * ESTIMATE, 20 * ESTIMATE, { overscan: 2 });
    const rendered = slots(parts);
    // 可视区第 50 条起 20 条（20*96=1920 高度覆盖 20 条），上下各多 2 条。
    expect(rendered).toEqual(Array.from({ length: 24 }, (_, i) => 48 + i));
    assertCovers(parts, count, []);
    // 占位高度 + 渲染高度 = 全部内容高度。
    expect(gapHeight(parts) + rendered.length * ESTIMATE).toBe(count * ESTIMATE);
  });

  it("实测高度参与占位高度的计算", () => {
    const count = 100;
    const heights = new Array<number>(count).fill(0);
    heights[0] = 40;
    heights[1] = 300;
    const parts = planWindow(count, heights, 0, 500, { overscan: 0 });
    assertCovers(parts, count, heights);
  });

  it("extra 里的下标即便在窗口外也渲染", () => {
    const count = 300;
    const parts = planWindow(count, [], 0, 300, { overscan: 1, extra: [150, 151, 299] });
    const rendered = slots(parts);
    expect(rendered).toContain(150);
    expect(rendered).toContain(151);
    expect(rendered).toContain(299);
    // extra 把列表切成多段渲染 + 多段占位，但整体仍不重不漏。
    assertCovers(parts, count, []);
    expect(parts.length).toBeGreaterThan(2);
  });

  it("extra 越界或重复不影响计划", () => {
    const count = 300;
    const parts = planWindow(count, [], 0, 300, { overscan: 1, extra: [-3, 3, 3, 299, 400] });
    const rendered = slots(parts);
    expect(rendered).toContain(3);
    expect(rendered).toContain(299);
    assertCovers(parts, count, []);
  });

  it("滚到最顶部与最底部时窗口贴边且不越界", () => {
    const count = 120;
    const top = slots(planWindow(count, [], 0, 5 * ESTIMATE, { overscan: 3 }));
    expect(top[0]).toBe(0);
    const end = slots(planWindow(count, [], count * ESTIMATE, 5 * ESTIMATE, { overscan: 3 }));
    expect(end[end.length - 1]).toBe(count - 1);
  });

  it("视口高度为 0 时也至少渲染一条", () => {
    const rendered = slots(planWindow(100, [], 50 * ESTIMATE, 0, { overscan: 0 }));
    expect(rendered.length).toBeGreaterThanOrEqual(1);
    expect(rendered).toContain(50);
  });
});

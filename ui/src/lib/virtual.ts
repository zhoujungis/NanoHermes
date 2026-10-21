/**
 * 消息区虚拟滚动的窗口计算。
 *
 * 超长会话里几百条消息全部挂载，既拖慢渲染也拖慢查找定位。这里把消息流拆成
 * 「真实渲染的窗口」+「按缓存高度撑起来的占位」，只挂载可视区附近的消息。
 *
 * 高度来源是调用方维护的测量缓存：测量过的用实测值，没测过的用估算值。估算
 * 只影响滚动条的粗略位置，用户滚到哪里就测到哪里，随后由调用方补偿 scrollTop。
 *
 * 纯函数、无 DOM 依赖，方便直接跑单测。
 */

/** 没测量过的消息按这个高度估算（约一条普通问答的高度）。 */
export const ESTIMATE = 96;

/** 消息不超过这个数时全量渲染：短会话没必要虚拟化，行为与普通列表一致。 */
export const FULL_LIMIT = 48;

export type PlanPart =
  /** 连续渲染的消息下标（升序）。 */
  | { kind: "slots"; indices: number[] }
  /** 不渲染的消息区间 [from, to)，用 height 撑起滚动条。 */
  | { kind: "gap"; from: number; to: number; height: number };

export interface PlanOpts {
  /** 未测量消息的估算高度。 */
  estimate?: number;
  /** 视口上下各多渲染几条，滚动时不至于露白。 */
  overscan?: number;
  /** 额外必须渲染的下标（例如查找命中所在的消息）。 */
  extra?: readonly number[];
  /** 消息数不超过它时全量渲染；传 Infinity 表示始终全量。 */
  full?: number;
}

/**
 * 计算要渲染的窗口。
 *
 * 返回的 parts 按下标升序铺满 [0, count)：slots 段真实渲染，gap 段是占位高度，
 * 两者交替出现，占位高度等于区间内各条消息高度（实测或估算）之和。
 */
export function planWindow(
  count: number,
  heights: readonly number[],
  scrollTop: number,
  viewportH: number,
  opts: PlanOpts = {},
): PlanPart[] {
  if (count <= 0) return [];
  const estimate = opts.estimate ?? ESTIMATE;
  const overscan = opts.overscan ?? 6;
  const full = opts.full ?? FULL_LIMIT;
  const h = (i: number): number => {
    const v = heights[i] ?? 0;
    return v > 0 ? v : estimate;
  };

  // 短会话直接全量渲染。
  if (count <= full) return [{ kind: "slots", indices: range(0, count) }];

  // 前缀和：cum[i] 是第 i 条消息顶边在滚动内容里的位置。
  const cum = new Array<number>(count + 1).fill(0);
  for (let i = 0; i < count; i++) cum[i + 1] = cum[i] + h(i);

  // 可视区覆盖到的第一条与最后一条消息。
  const top = Math.max(0, scrollTop);
  const bottom = top + Math.max(0, viewportH);
  let lo = 0;
  let hi = count;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (cum[mid + 1] > top) hi = mid;
    else lo = mid + 1;
  }
  const first = lo;
  lo = first;
  hi = count;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (cum[mid] < bottom) lo = mid + 1;
    else hi = mid;
  }
  const last = Math.max(first, lo - 1);

  const from = Math.max(0, first - overscan);
  const to = Math.min(count, last + 1 + overscan);

  const render = new Set<number>();
  for (let i = from; i < to; i++) render.add(i);
  for (const i of opts.extra ?? []) {
    if (i >= 0 && i < count) render.add(i);
  }

  // 把 [0, count) 切成渲染段与占位段交替的序列。
  const parts: PlanPart[] = [];
  let i = 0;
  while (i < count) {
    if (render.has(i)) {
      const indices: number[] = [];
      while (i < count && render.has(i)) {
        indices.push(i);
        i++;
      }
      parts.push({ kind: "slots", indices });
    } else {
      const g0 = i;
      let sum = 0;
      while (i < count && !render.has(i)) {
        sum += h(i);
        i++;
      }
      parts.push({ kind: "gap", from: g0, to: i, height: sum });
    }
  }
  return parts;
}

function range(from: number, to: number): number[] {
  const out: number[] = [];
  for (let i = from; i < to; i++) out.push(i);
  return out;
}

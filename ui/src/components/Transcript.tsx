import {
  Fragment,
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { rowButton } from "../lib/a11y";
import {
  clearHighlight,
  collectRanges,
  findHits,
  paintHighlight,
  scrollRangeIntoView,
  selectRange,
  supportsHighlight,
} from "../lib/find";
import { copyText, handleCopyClick } from "../lib/markdown";
import {
  activeTask,
  closeFind,
  editInComposer,
  openFolderPick,
  retryFrom,
  send,
  subscribeStreamText,
  taskIsCloud,
  useAppState,
  type Msg,
} from "../lib/store";
import { ESTIMATE, planWindow } from "../lib/virtual";
import { BrandMark, Icon, type IconName } from "./Icon";
import { Markdown } from "./Markdown";
import { FindBar } from "./FindBar";

const HERO_CARDS: Array<{ icon: IconName; h: string; prompt: string }> = [
  ["box", "了解项目结构", "用一句话介绍这个项目的结构和主要模块"],
  ["search", "排查潜在问题", "检查当前工作区，指出代码中可能存在的问题"],
  ["file", "总结目录职责", "列出 src 目录下的文件并总结每个文件的职责"],
  ["spark", "补充单元测试", "为 src/tools/search.rs 补充单元测试"],
].map(([icon, h, prompt]) => ({ icon: icon as IconName, h, prompt }));

/** 消息流：新消息自动滚到底部，用户向上翻阅时不打断。 */
export function Transcript() {
  const st = useAppState();
  const msgs = activeTask()?.msgs ?? [];
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  // 与 stick 同步的可见状态：只在「是否跟随底部」翻转时变化，避免滚动时频繁重渲染。
  const [atBottom, setAtBottom] = useState(true);

  // 虚拟滚动：每条消息的实测高度缓存（未测量的用估算值撑占位）。
  const heights = useRef<number[]>([]);
  // 测量回写后占位高度要重算，用版本号触发一次重渲染。
  const [ver, setVer] = useState(0);
  // 滚动位置与视口高度决定渲染窗口；滚动事件用 rAF 合并成每帧一次。
  const [view, setView] = useState({ top: 0, h: 0 });
  const raf = useRef(0);

  const syncView = () => {
    const el = scroller.current;
    if (!el) return;
    setView((v) =>
      v.top === el.scrollTop && v.h === el.clientHeight
        ? v
        : { top: el.scrollTop, h: el.clientHeight },
    );
  };

  // 查找状态留在组件里（不落盘）：查询串与当前命中序号。
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const hits = useMemo(
    () => (st.findOpen && query.trim() ? findHits(msgs, query) : []),
    [st.findOpen, query, msgs],
  );
  const total = hits.length;
  const index = total ? Math.min(cursor, total - 1) : -1;

  // 查找命中所在的消息必须在 DOM 里（find 按 [data-mi] 定位），一并扩进渲染窗口。
  const extra = useMemo(() => Array.from(new Set(hits.map((h) => h.msg))), [hits]);
  const plan = useMemo(
    () => planWindow(msgs.length, heights.current, view.top, view.h, { extra }),
    // ver 参与依赖：高度缓存回写后占位高度要跟着重算。
    [msgs.length, view, ver, extra],
  );
  const planSig = plan
    .map((p) => (p.kind === "slots" ? p.indices.join(",") : `g${p.from}`))
    .join(";");

  // 容器尺寸变化（窗口缩放、面板开合）会改变视口与消息高度，全部作废重测。
  useLayoutEffect(() => {
    syncView();
    const el = scroller.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      setVer((v) => v + 1);
      syncView();
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      if (raf.current) cancelAnimationFrame(raf.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const last = msgs[msgs.length - 1];
  // 最后一条是否正在流式增长：它就在列表末尾，自身变高不会挪动上方任何占位。
  const tailStreaming = !!last?.streaming;

  // 每次渲染后回测真实高度并回写缓存：占位高度越用越准。视口上方的消息高度被
  // 修正时下方内容会整体移动，同步挪 scrollTop 抵消掉，避免翻阅历史时抖动。
  // 流式期间直接跳过：末尾那条自己一直在长，这一帧量到的值下一帧就过期；而每帧
  // 遍历全部挂载消息做 offsetHeight / getBoundingClientRect（强制同步重排）再加
  // setVer 级联重渲染，正是「输出时整个界面卡住」的另一半来源。
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || tailStreaming) return;
    if (heights.current.length > msgs.length) heights.current.length = msgs.length;
    const box = el.getBoundingClientRect();
    let changed = false;
    for (const slot of el.querySelectorAll<HTMLElement>(".m-slot[data-si]")) {
      const i = Number(slot.dataset.si);
      const h = slot.offsetHeight;
      if (!Number.isInteger(i) || h <= 0) continue;
      const prev = heights.current[i] || ESTIMATE;
      if (Math.abs(prev - h) <= 1) continue;
      heights.current[i] = h;
      changed = true;
      if (!stick.current && slot.getBoundingClientRect().bottom <= box.top + 1) {
        el.scrollTop += h - prev;
      }
    }
    if (changed) setVer((v) => v + 1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [msgs.length, tailStreaming, ver, planSig]);

  // 滚动跟随走实时文本的独立通道：每帧只把滚动条钉到底，不引起任何 React 重渲染。
  // 挂在全局状态上的话，流式每一帧都要整树重渲染一次，跟随的代价远大于收益。
  useEffect(
    () =>
      subscribeStreamText(() => {
        const el = scroller.current;
        if (el && stick.current) el.scrollTop = el.scrollHeight;
      }),
    [],
  );

  /** 在当前命中之间循环跳转。 */
  const step = (delta: number) => {
    if (!total) return;
    setCursor(((index + delta) % total + total) % total);
  };
  const changeQuery = (v: string) => {
    setQuery(v);
    setCursor(0);
  };

  // 切换任务时强制回到最新位置；高度缓存是按消息下标记的，不能跨任务复用。
  useLayoutEffect(() => {
    stick.current = true;
    setAtBottom(true);
    heights.current = [];
    setVer((v) => v + 1);
  }, [st.activeId]);

  // 命中定位与高亮。放在「回到最新」之后：切换任务先归位，若此时正在查找，
  // 再跳到当前命中处；同一帧里先把 stick 关掉，否则会先跳到底部再跳回命中处。
  // 重绘只在「换了命中」或渲染窗口滑动时发生；流式期间的滚动跟随走独立通道，
  // 这里不会被每 40ms 的增量叫醒。
  // planSig：窗口滑动会把已高亮的命中卸载掉，窗口一变就要重绘高亮。
  const findSig = `${st.activeId}|${st.findOpen}|${query}|${index}|${total}|${msgs.length}|${(last?.t || "").length}|${planSig}`;
  const jumpKey = `${st.activeId}|${st.findOpen}|${query}|${index}|${total}`;
  const jumped = useRef("");
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !total) {
      clearHighlight();
      jumped.current = "";
      return;
    }
    const ranges = collectRanges(el, hits, query);
    paintHighlight(ranges, index);
    if (jumped.current === jumpKey) return;
    jumped.current = jumpKey;
    const range = ranges[index];
    if (!range) return;
    stick.current = false;
    setAtBottom(false);
    // 不支持高亮接口（旧内核）时用选区把当前这处标出来，滚动定位仍然有效。
    if (!supportsHighlight()) selectRange(range);
    scrollRangeIntoView(el, range);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [findSig]);

  // 只在「内容真的变了」时才回读 scrollHeight：滚动高度是同步重排，在输入框打字
  // 之类的无关更新不该触发它。流式期间的跟随由上面的通道订阅负责，走不到这里。
  const scrollSig = `${st.activeId}|${msgs.length}|${(last?.t || "").length}`;
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
    // ver 也要钉一次：占位高度被实测修正后 scrollHeight 会变。
  }, [scrollSig, ver]);

  const toBottom = () => {
    const el = scroller.current;
    if (!el) return;
    stick.current = true;
    setAtBottom(true);
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  };

  return (
    <div className="stream-wrap">
      <FindBar
        open={st.findOpen}
        focusTick={st.findTick}
        query={query}
        index={index}
        total={total}
        onQuery={changeQuery}
        onStep={step}
        onClose={closeFind}
      />
      <div
        className="scroller"
        ref={scroller}
        onScroll={() => {
          const el = scroller.current;
          if (!el) return;
          const near = el.scrollHeight - el.scrollTop - el.clientHeight < 140;
          stick.current = near;
          if (near !== atBottom) setAtBottom(near);
          // 滚动位置决定渲染窗口；rAF 合并，滚动时最多每帧重算一次。
          if (!raf.current) {
            raf.current = requestAnimationFrame(() => {
              raf.current = 0;
              syncView();
            });
          }
        }}
        onClick={(e) => handleCopyClick(e.target)}
      >
        <div className="stream">
          {msgs.length ? (
            // 只挂载窗口内的消息，其余用占位段撑起滚动条；入场动画只给最新几条，
            // 免得翻阅历史时每条都重放一遍。
            plan.map((p) =>
              p.kind === "gap" ? (
                <div key={"g" + p.from} className="v-gap" style={{ height: p.height }} />
              ) : (
                <Fragment key={"s" + p.indices[0]}>
                  {p.indices.map((i) => (
                    <div
                      key={i}
                      className={"m-slot" + (i >= msgs.length - 3 ? " fresh" : "")}
                      data-si={i}
                    >
                      <Message m={msgs[i]} index={i} />
                    </div>
                  ))}
                </Fragment>
              ),
            )
          ) : (
            <Hero />
          )}
        </div>
      </div>
      {msgs.length > 0 && (
        <button
          className={"jump" + (atBottom ? "" : " show")}
          title="回到最新"
          aria-hidden={atBottom}
          tabIndex={atBottom ? -1 : 0}
          onClick={toBottom}
        >
          <Icon name="arrowDown" />
        </button>
      )}
    </div>
  );
}

// 流式期间只有最后一条消息在变。不 memo 的话，每一帧都会把整条对话（可能上百条
// 消息）重新协调一遍，这才是「回答时电脑发卡」的主要来源。
const Message = memo(function Message({ m, index }: { m: Msg; index: number }) {
  /** 被点击放大的附件图片；null 表示没有打开查看层。 */
  const [zoom, setZoom] = useState<string | null>(null);

  // 查看层打开时，Esc 先关它（捕获阶段拦下，避免触发外层的浮层关闭链）。
  useEffect(() => {
    if (!zoom) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setZoom(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [zoom]);

  if (m.k === "user") {
    const text = (m.t || "").trim();
    return (
      <div className="msg user">
        <div className="uwrap">
          {m.imgs?.length ? (
            <div className="uimgs">
              {m.imgs.map((src, j) => (
                <img
                  key={j}
                  src={src}
                  alt={`附件 ${j + 1}`}
                  title="点击放大"
                  {...rowButton(() => setZoom(src))}
                  onClick={() => setZoom(src)}
                />
              ))}
            </div>
          ) : null}
          {text ? <div className="bubble" data-mi={index}>{m.t}</div> : null}
          <div className="uacts">
            {text ? <CopyButton text={m.t || ""} /> : null}
            <button
              title="载入输入框，修改后重新发送"
              onClick={() => editInComposer(m.t || "")}
            >
              <Icon name="pencil" />
              <span>编辑</span>
            </button>
          </div>
        </div>
        {zoom && (
          <div className="lightbox" onClick={() => setZoom(null)}>
            <img src={zoom} alt="附件放大预览" />
          </div>
        )}
      </div>
    );
  }
  if (m.k === "tool") return <ToolRow m={m} />;
  if (m.k === "note")
    return (
      <div className="note" data-mi={index}>
        {m.t}
      </div>
    );

  return (
    <div className="msg ai">
      <div className="av" aria-hidden="true">
        <BrandMark size={22} />
      </div>
      <div className="body" data-mi={index}>
        <Markdown text={m.t || ""} streaming={m.streaming} err={m.err} />
        {!m.streaming && (
          <div className="acts">
            <CopyButton text={m.t || ""} />
            <button title="重试" onClick={() => retryFrom(index)}>
              <Icon name="refresh" />
              <span>重试</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
});

/** 工具调用行：默认一行摘要，有结果时可展开看详情。 */
const ToolRow = memo(function ToolRow({ m }: { m: Msg }) {
  const [open, setOpen] = useState(false);
  const body = (m.out || "").trim();
  return (
    <div className={"act" + (body ? " x" : "")}>
      <button
        className="act-head"
        disabled={!body}
        title={body ? "展开/收起工具输出" : undefined}
        onClick={() => body && setOpen((o) => !o)}
      >
        <Icon name="terminal" />
        <span className="an">{m.name || "tool"}</span>
        <span className="as">{m.summary || ""}</span>
        {body && <Icon name="chev" className={"chev" + (open ? " open" : "")} />}
      </button>
      {open && <pre className="act-out">{body}</pre>}
    </div>
  );
});

function CopyButton({ text, label = "复制" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      title={label}
      onClick={() => {
        copyText(text);
        setDone(true);
        window.setTimeout(() => setDone(false), 1200);
      }}
    >
      <Icon name="copy" />
      <span>{done ? "已复制" : label}</span>
    </button>
  );
}

function Hero() {
  const st = useAppState();
  const task = activeTask();
  const cloud = taskIsCloud(task);
  const ws = (task?.workspace || st.settings?.workspace || "").trim();
  const cloudUrl = (st.settings?.cloud_url || "").trim();
  return (
    <div className="hero">
      <div className="mark" aria-hidden="true">
        <BrandMark size={34} />
      </div>
      <h1>开始一个任务</h1>
      <div className="sub">
        NanoHermes 可以读取文件、搜索代码、执行命令并直接修改你的项目，所有动作都会实时显示在右侧结果区。
      </div>
      <div className="ctx">
        <Icon name={cloud ? "cloud" : "laptop"} />
        <span title={cloud ? cloudUrl : ws}>
          {cloud ? cloudUrl || "未配置云服务地址" : ws || "未设置工作区"}
        </span>
        {!cloud && (
          <button
            className="mini"
            onClick={() => openFolderPick({ forTask: task?.id ?? null, initial: ws })}
          >
            <Icon name="folder" />
            {ws ? "更改" : "选择文件夹"}
          </button>
        )}
      </div>
      <div className="cards">
        {HERO_CARDS.map((c) => (
          <button key={c.h} className="card" onClick={() => void send(c.prompt)}>
            <div className="h">
              <Icon name={c.icon} />
              <span>{c.h}</span>
            </div>
            <div className="d">{c.prompt}</div>
          </button>
        ))}
      </div>
    </div>
  );
}

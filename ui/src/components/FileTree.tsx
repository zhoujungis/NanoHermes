import { useCallback, useEffect, useMemo, useState } from "react";
import { rowButton } from "../lib/a11y";
import {
  activeTask,
  addRef,
  openFolderPick,
  openPreview,
  taskIsCloud,
  toast,
  useAppState,
} from "../lib/store";
import { errText, invoke } from "../lib/tauri";
import { Icon, type IconName } from "./Icon";

/** 目录树节点。`path` 是相对工作区的 POSIX 风格路径。 */
interface Node {
  name: string;
  path: string;
  dir: boolean;
  children: Node[];
}

/** 由扁平路径列表拼出树。目录排在文件前面，同级按名称排序。 */
function buildTree(files: string[]): Node[] {
  const roots: Node[] = [];
  const index = new Map<string, Node>();
  for (const f of files) {
    const parts = f.split("/");
    let parent = "";
    let level = roots;
    for (let i = 0; i < parts.length; i++) {
      const name = parts[i];
      const last = i === parts.length - 1;
      const path = parent ? `${parent}/${name}` : name;
      let node = index.get(path);
      if (!node) {
        node = { name, path, dir: !last, children: [] };
        index.set(path, node);
        level.push(node);
      }
      level = node.children;
      parent = path;
    }
  }
  sortLevel(roots);
  return roots;
}

function sortLevel(nodes: Node[]): void {
  nodes.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
  for (const n of nodes) if (n.dir) sortLevel(n.children);
}

function fileIcon(path: string): IconName {
  if (/\.(rs|ts|tsx|js|jsx|py|go|java|c|cpp|h|hpp|cs|rb|php|swift|kt)$/i.test(path)) return "file";
  if (/\.(md|txt|json|toml|yaml|yml|ini|cfg|env)$/i.test(path)) return "edit";
  return "file";
}

/** 工作区文件树：浏览整个工作区，点文件看内容，`+` 把路径加进提问引用。 */
export function FileTree() {
  const st = useAppState();
  const task = activeTask();
  const cloud = taskIsCloud(task);
  const ws = (task?.workspace || st.settings?.workspace || "").trim();
  const [files, setFiles] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [open, setOpen] = useState<Set<string>>(new Set());

  const load = useCallback(() => {
    if (cloud) return;
    if (!ws) {
      setFiles([]);
      setErr(null);
      return;
    }
    setErr(null);
    invoke<string[]>("list_workspace_files")
      .then((r) => {
        setFiles(r);
        // 默认展开第一层目录，既能看到结构又不至于一次铺开几千行。
        setOpen(new Set(buildTree(r).filter((n) => n.dir).map((n) => n.path)));
      })
      .catch((e) => {
        setFiles([]);
        setErr(errText(e));
      });
  }, [ws, cloud]);

  useEffect(() => {
    load();
  }, [load]);

  const tree = useMemo(() => buildTree(files || []), [files]);

  const toggle = (path: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  if (cloud) {
    return (
      <div className="as-empty">
        <Icon name="cloud" />
        云端任务的工作区在远端服务器上，这里无法浏览本地文件。
      </div>
    );
  }

  if (!ws) {
    return (
      <div className="as-empty">
        <Icon name="folder" />
        这个任务还没有设置工作区文件夹。
        <div className="as-empty-act">
          <button
            className="mini"
            onClick={() => openFolderPick({ forTask: task?.id ?? null, initial: "" })}
          >
            <Icon name="folder" />
            选择文件夹
          </button>
        </div>
      </div>
    );
  }

  if (err) {
    return (
      <div className="as-empty">
        <Icon name="folder" />
        {err}
        <div className="as-empty-act">
          <button className="mini" onClick={load}>
            <Icon name="refresh" />
            重新加载
          </button>
        </div>
      </div>
    );
  }

  if (!files) return <div className="as-empty">正在读取工作区…</div>;

  if (!tree.length) {
    return (
      <div className="as-empty">
        <Icon name="folder" />
        工作区里没有可显示的文件。
      </div>
    );
  }

  const previewPath = st.preview?.path ?? "";

  return (
    <div className="tree">
      <div className="tree-bar">
        <span className="tp" title={ws}>
          {ws.split(/[\\/]/).filter(Boolean).pop() || ws}
        </span>
        <span className="sp" />
        <button className="tbtn sm" title="重新加载" onClick={load}>
          <Icon name="refresh" />
        </button>
      </div>
      <Rows
        nodes={tree}
        depth={0}
        open={open}
        toggle={toggle}
        previewPath={previewPath}
      />
    </div>
  );
}

function Rows({
  nodes,
  depth,
  open,
  toggle,
  previewPath,
}: {
  nodes: Node[];
  depth: number;
  open: Set<string>;
  toggle: (path: string) => void;
  previewPath: string;
}) {
  return (
    <>
      {nodes.map((n) =>
        n.dir ? (
          <div key={n.path}>
            <div
              className="trow dir"
              style={{ paddingLeft: 8 + depth * 13 }}
              title={n.path}
              {...rowButton(() => toggle(n.path))}
              onClick={() => toggle(n.path)}
            >
              <Icon name="chev" className={"chev" + (open.has(n.path) ? " open" : "")} />
              <Icon name="folder" />
              <span className="tn">{n.name}</span>
            </div>
            {open.has(n.path) && (
              <Rows
                nodes={n.children}
                depth={depth + 1}
                open={open}
                toggle={toggle}
                previewPath={previewPath}
              />
            )}
          </div>
        ) : (
          <div
            key={n.path}
            className={"trow" + (previewPath === n.path ? " on" : "")}
            style={{ paddingLeft: 8 + depth * 13 + 18 }}
            title={n.path}
            {...rowButton(() => void openPreview(n.path))}
            onClick={() => void openPreview(n.path)}
          >
            <Icon name={fileIcon(n.path)} />
            <span className="tn">{n.name}</span>
            <button
              className="tadd"
              title="加入提问引用"
              onClick={(e) => {
                e.stopPropagation();
                addRef(n.path);
                toast("已引用：" + n.path, "ok");
              }}
            >
              <Icon name="plus" />
            </button>
          </div>
        ),
      )}
    </>
  );
}

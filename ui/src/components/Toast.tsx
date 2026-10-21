import { useEffect, useState } from "react";
import { useAppState, type ToastKind } from "../lib/store";
import { Icon, type IconName } from "./Icon";

const ICON: Record<ToastKind, IconName> = { info: "info", ok: "check", err: "alert" };

/** 底部浮层提示。文案保留到淡出结束，避免消失瞬间文字被清空。 */
export function Toast() {
  const { toast, toastKind } = useAppState();
  const [shown, setShown] = useState<{ text: string; kind: ToastKind }>({
    text: "",
    kind: "info",
  });

  useEffect(() => {
    if (toast) setShown({ text: toast, kind: toastKind });
  }, [toast, toastKind]);

  return (
    <div className={"toast " + shown.kind + (toast ? " show" : "")} role="status" aria-live="polite">
      <Icon name={ICON[shown.kind]} />
      <span>{shown.text}</span>
    </div>
  );
}

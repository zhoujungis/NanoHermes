import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { initTheme } from "./lib/theme";
import "./styles/shell.css";

initTheme();

// 开发环境暴露流式压测入口（生产构建里这段会被静态消除）。
if (import.meta.env.DEV) {
  void import("./lib/store").then((s) => {
    (window as unknown as Record<string, unknown>).__nhBench = s.devBench;
  });
}

const container = document.getElementById("root");
if (!container) throw new Error("#root 未找到");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

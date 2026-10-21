/**
 * 代码着色引擎的唯一入口：highlight.js core + 按需注册语言。
 *
 * 不用官方 common 版（自带 37 种语言，压进主 chunk 后体积超线）；只注册界面
 * 会真正着色的语言，其余语言由 langOf 判定为「拿不准」，保持纯文本。
 * code.ts 与 markdown.ts 共用这里的同一个实例，避免重复注册。
 */
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import less from "highlight.js/lib/languages/less";
import markdown from "highlight.js/lib/languages/markdown";
import php from "highlight.js/lib/languages/php";
import powershell from "highlight.js/lib/languages/powershell";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import scss from "highlight.js/lib/languages/scss";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const LANGS: Array<[string, (typeof rust)]> = [
  ["bash", bash],
  ["c", c],
  ["cpp", cpp],
  ["csharp", csharp],
  ["css", css],
  ["go", go],
  ["ini", ini],
  ["java", java],
  ["javascript", javascript],
  ["json", json],
  ["kotlin", kotlin],
  ["less", less],
  ["markdown", markdown],
  ["php", php],
  ["powershell", powershell],
  ["python", python],
  ["ruby", ruby],
  ["rust", rust],
  ["scss", scss],
  ["sql", sql],
  ["swift", swift],
  ["typescript", typescript],
  ["xml", xml],
  ["yaml", yaml],
];

// 注册是模块级一次性动作；Vitest 每个测试文件独立模块图，重复注册也安全。
if (!hljs.getLanguage("rust")) {
  LANGS.forEach(([name, def]) => hljs.registerLanguage(name, def));
}

export { hljs };

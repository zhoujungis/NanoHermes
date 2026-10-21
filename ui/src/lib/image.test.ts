/**
 * 图片附件模块的测试：体积文案、文件筛选、以及各种拒绝/降级路径。
 *
 * 真正的缩放要走 canvas，jsdom 不实现它；这里用 stub 控制 createImageBitmap 的
 * 返回值，覆盖「原样直发」「需要缩放但环境不支持」和各类拒绝分支。
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { formatBytes, imageFilesFrom, MAX_ATTACHMENTS, toAttachment } from "./image";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("formatBytes", () => {
  it("按量级切换单位", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(10240)).toBe("10 KB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
  });
});

describe("imageFilesFrom", () => {
  it("空列表返回空数组", () => {
    expect(imageFilesFrom(null)).toEqual([]);
  });

  it("只挑出图片文件", () => {
    const png = new File(["x"], "a.png", { type: "image/png" });
    const txt = new File(["x"], "a.txt", { type: "text/plain" });
    const out = imageFilesFrom([png, txt] as unknown as FileList);
    expect(out).toEqual([png]);
  });

  it("从 DataTransferItem 里取出图片文件", () => {
    const png = new File(["x"], "a.png", { type: "image/png" });
    const items = [{ kind: "file", getAsFile: () => png }];
    const out = imageFilesFrom(items as unknown as DataTransferItemList);
    expect(out).toEqual([png]);
  });
});

describe("toAttachment", () => {
  it("非图片文件被拒绝", async () => {
    const file = new File(["x"], "a.txt", { type: "text/plain" });
    await expect(toAttachment(file)).rejects.toThrow("只能附加图片文件");
  });

  it("超过体积上限的图片被拒绝", async () => {
    const file = new File([""], "big.png", { type: "image/png" });
    Object.defineProperty(file, "size", { value: 13 * 1024 * 1024 });
    await expect(toAttachment(file)).rejects.toThrow("图片过大");
  });

  it("无法解码的图片带原因抛出", async () => {
    vi.stubGlobal("createImageBitmap", () => Promise.reject(new Error("bad")));
    const file = new File(["x"], "bad.png", { type: "image/png" });
    await expect(toAttachment(file)).rejects.toThrow("无法解码");
  });

  it("本来就小的图片原样直发，不做重编码", async () => {
    vi.stubGlobal("createImageBitmap", async () => ({
      width: 100,
      height: 80,
      close: () => {},
    }));
    const file = new File(["x"], "s.png", { type: "image/png" });

    const a = await toAttachment(file);
    expect(a.w).toBe(100);
    expect(a.h).toBe(80);
    expect(a.name).toBe("s.png");
    expect(a.url.startsWith("data:image/png;base64,")).toBe(true);
    expect(a.bytes).toBeGreaterThan(0);
  });

  it("需要缩放的图片走 canvas；环境不支持时明确报错", async () => {
    vi.stubGlobal("createImageBitmap", async () => ({
      width: 2048,
      height: 1024,
      close: () => {},
    }));
    const file = new File([""], "wide.png", { type: "image/png" });
    await expect(toAttachment(file)).rejects.toThrow("不支持图片压缩");
  });
});

describe("常量", () => {
  it("单次附件上限与后端保持一致", () => {
    expect(MAX_ATTACHMENTS).toBe(4);
  });
});

/**
 * 图片附件的读取与压缩。
 *
 * 原图（截图、手机照片）动辄几 MB，直接 base64 塞进请求体既拖慢 IPC，也会撑大
 * 任务会话文件。这里统一缩到最长边 1024 再编码：视觉信息基本无损，体积通常降到
 * 几百 KB。截图这类少色图先用 PNG 保住文字锐利，超过阈值再退回 JPEG 控制体积。
 */

/** 一次最多附加的图片数量（与后端 MAX_ATTACHED_IMAGES 保持一致）。 */
export const MAX_ATTACHMENTS = 4;

/** 缩放后最长边。 */
const MAX_EDGE = 1024;
/** 单张原图体积上限；超过就拒绝，避免读取阶段卡死。 */
const MAX_SOURCE_BYTES = 12 * 1024 * 1024;
/** PNG 结果超过这个长度就改用 JPEG。 */
const PNG_BUDGET = 800_000;

export interface Attachment {
  /** 文件名，仅用于展示与提示。 */
  name: string;
  /** `data:image/...;base64,...`，直接作为 image_url 交给模型。 */
  url: string;
  w: number;
  h: number;
  /** 编码后的近似字节数。 */
  bytes: number;
}

/** 人类可读的体积文案。 */
export function formatBytes(n: number): string {
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(n < 10240 ? 1 : 0) + " KB";
  return (n / 1024 / 1024).toFixed(1) + " MB";
}

/** data URL 的实际字节数（base64 每 4 字符 3 字节，末尾可能补 padding）。 */
function dataUrlBytes(url: string): number {
  const comma = url.indexOf(",");
  if (comma < 0) return 0;
  const b64 = url.length - comma - 1;
  return Math.floor((b64 * 3) / 4);
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(String(fr.result || ""));
    fr.onerror = () => reject(new Error(`读取 ${file.name} 失败`));
    fr.readAsDataURL(file);
  });
}

/**
 * 把一张图片变成可发送的附件。非图片、超大文件、无法解码的图片都会抛出带原因的
 * 错误，由调用方转成提示文案。
 */
export async function toAttachment(file: File): Promise<Attachment> {
  if (!file.type.startsWith("image/")) {
    throw new Error("只能附加图片文件（PNG / JPEG / GIF / WEBP）");
  }
  if (file.size > MAX_SOURCE_BYTES) {
    throw new Error(`图片过大：${formatBytes(file.size)}，请压缩后再试`);
  }

  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new Error(`${file.name} 无法解码，可能不是有效的图片`);
  }

  const scale = Math.min(1, MAX_EDGE / Math.max(bitmap.width, bitmap.height));
  const w = Math.max(1, Math.round(bitmap.width * scale));
  const h = Math.max(1, Math.round(bitmap.height * scale));

  // 本来就小、又不需要缩放的图直接原样发送，省掉一次有损重编码。
  if (scale === 1 && file.size <= 250_000) {
    const url = await readAsDataUrl(file);
    bitmap.close();
    return { name: file.name, url, w, h, bytes: dataUrlBytes(url) };
  }

  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    bitmap.close();
    throw new Error("当前环境不支持图片压缩");
  }
  ctx.drawImage(bitmap, 0, 0, w, h);
  bitmap.close();

  let url = canvas.toDataURL("image/png");
  if (url.length > PNG_BUDGET) url = canvas.toDataURL("image/jpeg", 0.85);
  return { name: file.name, url, w, h, bytes: dataUrlBytes(url) };
}

/** 从剪贴板/拖拽事件里挑出图片文件（FileList 与 DataTransferItemList 都可索引）。 */
export function imageFilesFrom(list: FileList | DataTransferItemList | null): File[] {
  if (!list) return [];
  const items = list as unknown as ArrayLike<File | DataTransferItem>;
  const out: File[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!item) continue;
    const file =
      item instanceof File
        ? item
        : item.kind === "file"
          ? item.getAsFile()
          : null;
    if (file && file.type.startsWith("image/")) out.push(file);
  }
  return out;
}

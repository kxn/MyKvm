// 剪贴板：粘贴经 navigator.clipboard.readText → noVNC clipboardPasteFrom；
// 复制截图经 ClipboardItem 写入。

export async function pasteFromClipboard(rfb) {
  if (!rfb || typeof navigator.clipboard?.readText !== "function") {
    throw new Error("clipboard-read unavailable");
  }
  const text = await navigator.clipboard.readText();
  rfb.clipboardPasteFrom(text);
  return text;
}

export async function copyBlobToClipboard(blob) {
  if (typeof navigator.clipboard?.write !== "function" || typeof ClipboardItem === "undefined") {
    throw new Error("clipboard-write unavailable");
  }
  await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
}

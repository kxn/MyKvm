// 剪贴板：粘贴经 navigator.clipboard.readText → noVNC clipboardPasteFrom；
// 复制截图先转 PNG 再经 ClipboardItem 写入（浏览器写剪贴板只支持 image/png）。

export async function pasteFromClipboard(rfb) {
  if (!rfb || typeof navigator.clipboard?.readText !== "function") {
    throw new Error("clipboard-read unavailable");
  }
  const text = await navigator.clipboard.readText();
  rfb.clipboardPasteFrom(text);
  return text;
}

export async function jpegToPngBlob(jpegBlob) {
  const bitmap = await createImageBitmap(jpegBlob);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const ctx = canvas.getContext("2d");
    ctx.drawImage(bitmap, 0, 0);
    return await new Promise((resolve, reject) => {
      canvas.toBlob(
        (blob) => (blob ? resolve(blob) : reject(new Error("png encode failed"))),
        "image/png",
      );
    });
  } finally {
    bitmap.close();
  }
}

export async function copyJpegToClipboard(jpegBlob) {
  const pngBlob = await jpegToPngBlob(jpegBlob);
  if (typeof navigator.clipboard?.write !== "function" || typeof ClipboardItem === "undefined") {
    throw new Error("clipboard-write unavailable");
  }
  await navigator.clipboard.write([new ClipboardItem({ "image/png": pngBlob })]);
}

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
  // 优先使用 Clipboard API（需要 HTTPS 或 localhost）
  if (typeof navigator.clipboard?.write === "function" && typeof ClipboardItem !== "undefined") {
    await navigator.clipboard.write([new ClipboardItem({ "image/png": pngBlob })]);
    return;
  }
  // Fallback：使用 document.execCommand('copy')（HTTP 环境可用）
  const url = URL.createObjectURL(pngBlob);
  try {
    const img = document.createElement("img");
    img.src = url;
    document.body.appendChild(img);
    const range = document.createRange();
    range.selectNode(img);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    document.execCommand("copy");
    selection.removeAllRanges();
    document.body.removeChild(img);
  } finally {
    URL.revokeObjectURL(url);
  }
}

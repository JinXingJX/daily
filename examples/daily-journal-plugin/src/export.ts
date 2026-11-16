export function download(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url; link.download = name; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}
export async function quoteCards(title: string, body: string, date: string): Promise<void> {
  const canvas = document.createElement("canvas");
  canvas.width = 1080; canvas.height = 1350;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is unavailable.");
  function wrap(text: string, width: number): string[] {
    return text.split("\n").flatMap(paragraph => {
      const lines: string[] = []; let line = "";
      for (const char of Array.from(paragraph)) {
        if (ctx!.measureText(line + char).width > width && line) { lines.push(line); line = ""; }
        line += char;
      }
      lines.push(line); return lines;
    });
  }
  ctx.font = '600 58px "PingFang SC", "Segoe UI", sans-serif';
  const titleLines = wrap(title, 880).slice(0, 4);
  ctx.font = '38px "PingFang SC", "Segoe UI", sans-serif';
  const lines = wrap(body, 880);
  const start = 180 + titleLines.length * 78;
  const perPage = Math.max(8, Math.floor((1200 - start) / 62));
  const pages = Math.max(1, Math.ceil(lines.length / perPage));
  for (let page = 0; page < pages; page++) {
    ctx.fillStyle = "#f3f6fc"; ctx.fillRect(0, 0, 1080, 1350);
    ctx.fillStyle = "#2359a8"; ctx.fillRect(100, 90, 48, 6);
    ctx.fillStyle = "#202c42"; ctx.font = '600 58px "PingFang SC", "Segoe UI", sans-serif';
    titleLines.forEach((line, i) => ctx.fillText(line, 100, 190 + i * 78));
    ctx.font = '38px "PingFang SC", "Segoe UI", sans-serif'; ctx.fillStyle = "#34435c";
    lines.slice(page * perPage, (page + 1) * perPage).forEach((line, i) => ctx.fillText(line, 100, start + 50 + i * 62));
    ctx.font = '26px "PingFang SC", sans-serif'; ctx.fillStyle = "#617087";
    ctx.fillText(date, 100, 1270); ctx.textAlign = "right"; ctx.fillText(`${page + 1} / ${pages}`, 980, 1270); ctx.textAlign = "left";
    const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob(value => value ? resolve(value) : reject(new Error("Image export failed.")), "image/png"));
    download(blob, `journal-${date}-${page + 1}.png`);
  }
}

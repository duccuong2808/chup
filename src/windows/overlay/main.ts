// Overlay chọn vùng (SPEC 5.1). Mỗi màn hình 1 cửa sổ; toạ độ gửi lên là CSS px
// tương đối cửa sổ, Rust tự quy đổi sang physical theo màn hình của overlay này.

import "./style.scss";
import { ipc, on } from "../../shared/ipc";

const dim = document.getElementById("dim")!;
const sel = document.getElementById("sel")!;
const sizeEl = document.getElementById("size")!;
const hint = document.getElementById("hint")!;

let start: { x: number; y: number } | null = null;
let rect = { x: 0, y: 0, w: 0, h: 0 };

function reset() {
  start = null;
  rect = { x: 0, y: 0, w: 0, h: 0 };
  sel.hidden = true;
  sizeEl.hidden = true;
  dim.hidden = false;
  hint.hidden = false;
}

const clampX = (v: number) => Math.min(Math.max(v, 0), window.innerWidth);
const clampY = (v: number) => Math.min(Math.max(v, 0), window.innerHeight);

function render(cx: number, cy: number) {
  if (!start) return;
  const x = Math.min(start.x, cx);
  const y = Math.min(start.y, cy);
  rect = { x, y, w: Math.abs(cx - start.x), h: Math.abs(cy - start.y) };

  Object.assign(sel.style, {
    left: `${rect.x}px`,
    top: `${rect.y}px`,
    width: `${rect.w}px`,
    height: `${rect.h}px`,
  });
  sel.hidden = false;
  dim.hidden = true;

  const dpr = window.devicePixelRatio || 1;
  sizeEl.textContent = `${Math.round(rect.w * dpr)} × ${Math.round(rect.h * dpr)}`;
  sizeEl.hidden = false;
  const labelY = rect.y + rect.h + 6 + 22 > window.innerHeight ? rect.y - 26 : rect.y + rect.h + 6;
  sizeEl.style.left = `${Math.min(rect.x, window.innerWidth - 110)}px`;
  sizeEl.style.top = `${Math.max(labelY, 2)}px`;
}

function cancel() {
  reset();
  void ipc.selectionCancelled();
}

window.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  start = { x: clampX(e.clientX), y: clampY(e.clientY) };
  hint.hidden = true;
  document.body.setPointerCapture(e.pointerId);
});

window.addEventListener("pointermove", (e) => {
  if (start) render(clampX(e.clientX), clampY(e.clientY));
});

window.addEventListener("pointerup", (e) => {
  if (e.button !== 0 || !start) return;
  render(clampX(e.clientX), clampY(e.clientY));
  const dpr = window.devicePixelRatio || 1;
  const r = rect;
  reset();
  if (r.w * dpr < 4 || r.h * dpr < 4) {
    void ipc.selectionCancelled();
  } else {
    void ipc.regionSelected(r);
  }
});

window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  cancel();
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    cancel();
  }
});

void on("selection:start", reset);
reset();

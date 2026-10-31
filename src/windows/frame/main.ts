// Viền vùng chọn (SPEC 5.2, 5.3). Di chuyển/resize dùng thao tác native của OS;
// Rust nghe event Moved/Resized của cửa sổ để cập nhật region.

import "./style.scss";
import { getCurrentWindow, type Window } from "@tauri-apps/api/window";
import { ipc, on } from "../../shared/ipc";
import type { FrameState } from "../../shared/types";

const win = getCurrentWindow();

type ResizeDirection = Parameters<Window["startResizeDragging"]>[0];

function apply(s: FrameState) {
  document.documentElement.style.setProperty("--m", `${s.marginCss}px`);
  document.body.classList.toggle("faded", s.mode === "faded");
}

document.querySelectorAll<HTMLElement>(".strip").forEach((el) => {
  el.addEventListener("pointerdown", (e) => {
    if (e.button === 0) void win.startDragging();
  });
});

document.querySelectorAll<HTMLElement>(".handle").forEach((el) => {
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    void win.startResizeDragging(el.dataset.dir as ResizeDirection);
  });
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void ipc.closeRegion();
  else if (e.key === "Enter") void ipc.captureNow();
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

void on("frame:state", apply);
void ipc.frameState().then(apply);

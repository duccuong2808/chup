// Stack thumbnail (SPEC 5.4). Cửa sổ không lấy focus; kích thước cửa sổ do Rust đặt
// theo kích thước nội dung mà trang này báo lên (chiều cao 0 = ẩn cửa sổ).

import "./style.scss";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { errorMessage, ipc, on } from "../../shared/ipc";
import { icons } from "../../shared/icons";
import type { Shot, StackState, Toast } from "../../shared/types";

const STACK_W = 184; // = wm::STACK_WIDTH_CSS
const TAB_W = 28;
const TAB_H = 62; // 56 + 6 lề dưới
const DRAG_THRESHOLD = 4;

const wrap = document.getElementById("wrap")!;
const header = document.getElementById("header")!;
const list = document.getElementById("list")!;
const toastsEl = document.getElementById("toasts")!;
const tab = document.getElementById("tab") as HTMLButtonElement;
const collapseBtn = document.getElementById("collapse") as HTMLButtonElement;
const clearBtn = document.getElementById("clear") as HTMLButtonElement;

let state: StackState = { shots: [], edge: "right" };
let collapsed = false;
let hovering = false;
let leaveTimer: number | undefined;
let toasts: Array<Toast & { id: number }> = [];
let toastSeq = 0;
let lastReported = "";

// ---------- render ----------

function thumbHeight(s: Shot): number {
  return Math.max(40, Math.min(120, Math.round((160 * s.height) / Math.max(s.width, 1))));
}

function cardFor(s: Shot): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "card";
  li.dataset.id = s.id;
  li.style.height = `${thumbHeight(s)}px`;
  li.title = s.path;

  const img = document.createElement("img");
  img.src = s.thumbDataUrl;
  img.alt = "";
  img.draggable = false;

  const meta = document.createElement("span");
  meta.className = "meta";
  meta.textContent = `${s.width} × ${s.height}`;

  const actions = document.createElement("div");
  actions.className = "actions";
  actions.append(
    actionButton(icons.copy, "Copy ảnh", () => ipc.shotCopy(s.id)),
    actionButton(icons.folder, "Mở thư mục chứa ảnh", () => ipc.shotReveal(s.id)),
    actionButton(icons.close, "Bỏ khỏi stack (file vẫn giữ)", () => ipc.stackRemove(s.id)),
  );

  li.append(img, meta, actions);
  li.addEventListener("pointerdown", (e) => onCardPointerDown(e, s.id));
  return li;
}

function actionButton(svg: string, title: string, run: () => Promise<unknown>): HTMLButtonElement {
  const b = document.createElement("button");
  b.innerHTML = svg;
  b.title = title;
  b.addEventListener("pointerdown", (e) => e.stopPropagation());
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    run().catch((err) => addToast({ kind: "error", message: errorMessage(err) }));
  });
  return b;
}

function renderList() {
  // Giữ lại node cũ theo id để hover không bị nháy khi stack đổi.
  const existing = new Map<string, HTMLLIElement>();
  list.querySelectorAll<HTMLLIElement>(".card").forEach((li) => existing.set(li.dataset.id!, li));
  const nodes = state.shots.map((s) => existing.get(s.id) ?? cardFor(s));
  list.replaceChildren(...nodes);
}

function renderToasts() {
  toastsEl.replaceChildren(
    ...toasts.map((t) => {
      const el = document.createElement("div");
      el.className = `toast ${t.kind}`;
      el.textContent = t.message;
      el.title = "Bấm để đóng";
      el.addEventListener("click", () => removeToast(t.id));
      return el;
    }),
  );
}

function render() {
  const hasShots = state.shots.length > 0;
  document.body.classList.toggle("edge-right", state.edge === "right");
  document.body.classList.toggle("edge-left", state.edge === "left");

  collapseBtn.innerHTML = state.edge === "right" ? icons.collapse : icons.expand;
  tab.innerHTML = `${state.edge === "right" ? icons.expand : icons.collapse}<span>${state.shots.length}</span>`;

  list.hidden = collapsed || !hasShots;
  header.hidden = !(hovering && !collapsed && hasShots);
  tab.hidden = !(collapsed && hasShots);
  wrap.hidden = list.hidden && toasts.length === 0;
  wrap.style.bottom = tab.hidden ? "0" : `${TAB_H}px`;

  report();
}

/** Báo kích thước nội dung (CSS px) để Rust đặt kích thước & vị trí cửa sổ. */
function report() {
  requestAnimationFrame(() => {
    let w = 0;
    let h = 0;
    if (!wrap.hidden) {
      w = STACK_W;
      h = Math.ceil(wrap.getBoundingClientRect().height);
    }
    if (!tab.hidden) {
      w = Math.max(w, TAB_W);
      h += TAB_H;
    }
    const key = `${w}x${h}`;
    if (key === lastReported) return;
    lastReported = key;
    void ipc.stackReportSize(w, h);
  });
}

// ---------- kéo thả / click (SPEC 5.4) ----------

let pending: { id: string; x: number; y: number; el: Element; pointerId: number } | null = null;

function onCardPointerDown(e: PointerEvent, id: string) {
  if (e.button !== 0) return;
  const el = e.currentTarget as Element;
  // Capture để vẫn nhận pointermove khi chuột ra khỏi cửa sổ nhỏ của stack.
  el.setPointerCapture(e.pointerId);
  pending = { id, x: e.screenX, y: e.screenY, el, pointerId: e.pointerId };
}

function takePending() {
  const p = pending;
  pending = null;
  if (p && p.el.hasPointerCapture(p.pointerId)) p.el.releasePointerCapture(p.pointerId);
  return p;
}

window.addEventListener("pointermove", (e) => {
  if (!pending) return;
  if (Math.hypot(e.screenX - pending.x, e.screenY - pending.y) < DRAG_THRESHOLD) return;
  const p = takePending();
  if (p) void beginDrag(p.id);
});

window.addEventListener("pointerup", () => {
  const p = takePending();
  if (!p) return;
  ipc.shotOpen(p.id).catch((err) => addToast({ kind: "error", message: errorMessage(err) }));
});

window.addEventListener("pointercancel", () => void takePending());

async function beginDrag(id: string) {
  const shot = state.shots.find((s) => s.id === id);
  if (!shot) return;
  try {
    // File bị xoá bên ngoài -> Rust tự bỏ shot khỏi stack và báo lỗi.
    if (!(await ipc.shotExists(id))) return;
    await startDrag({ item: [shot.path], icon: shot.thumbPath });
  } catch (err) {
    addToast({ kind: "error", message: `Không kéo được ảnh: ${errorMessage(err)}` });
  }
}

// ---------- header, thu gọn ----------

document.documentElement.addEventListener("mouseenter", () => {
  window.clearTimeout(leaveTimer);
  if (!hovering) {
    hovering = true;
    render();
  }
});

document.documentElement.addEventListener("mouseleave", () => {
  window.clearTimeout(leaveTimer);
  leaveTimer = window.setTimeout(() => {
    hovering = false;
    render();
  }, 400);
});

collapseBtn.addEventListener("click", () => {
  collapsed = true;
  render();
});

tab.addEventListener("click", () => {
  collapsed = false;
  render();
});

clearBtn.addEventListener("click", () => {
  ipc.stackClear().catch((err) => addToast({ kind: "error", message: errorMessage(err) }));
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

// ---------- toast ----------

function addToast(t: Toast) {
  const id = ++toastSeq;
  toasts = [...toasts.slice(-2), { ...t, id }];
  renderToasts();
  render();
  window.setTimeout(() => removeToast(id), t.kind === "error" ? 7000 : 2500);
}

function removeToast(id: number) {
  const before = toasts.length;
  toasts = toasts.filter((t) => t.id !== id);
  if (toasts.length !== before) {
    renderToasts();
    render();
  }
}

// ---------- state từ Rust ----------

function applyState(next: StackState) {
  // Có ảnh mới -> tự mở rộng nếu đang thu gọn.
  const prevIds = new Set(state.shots.map((s) => s.id));
  if (next.shots.some((s) => !prevIds.has(s.id))) collapsed = false;
  state = next;
  renderList();
  render();
}

void on("stack:changed", applyState);
void on("toast", addToast);
void ipc.stackState().then(applyState);

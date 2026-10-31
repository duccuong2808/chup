// Cài đặt (SPEC 5.6). Mọi thay đổi lưu ngay; Rust trả về state mới sau mỗi lần lưu.

import "./style.scss";
import { errorMessage, ipc, on } from "../../shared/ipc";
import type { Config, DeepPartial, HotkeyName, SettingsState } from "../../shared/types";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const saveDir = $<HTMLElement>("saveDir");
const pattern = $<HTMLInputElement>("pattern");
const preview = $<HTMLElement>("preview");
const copyToClipboard = $<HTMLInputElement>("copyToClipboard");
const afterCaptureNormal = $<HTMLSelectElement>("afterCaptureNormal");
const stackEdge = $<HTMLSelectElement>("stackEdge");
const stackMonitor = $<HTMLSelectElement>("stackMonitor");
const launchAtStartup = $<HTMLInputElement>("launchAtStartup");
const hideUi = $<HTMLInputElement>("hideUiDuringCapture");
const exclusionNote = $<HTMLElement>("exclusionNote");
const status = $<HTMLElement>("status");
const hotkeyInputs = Array.from(document.querySelectorAll<HTMLInputElement>("input.hotkey"));

let current: SettingsState | null = null;
let recording: HTMLInputElement | null = null;
/** Phím chính đã thấy keydown trong lần ghi hiện tại (theo `KeyboardEvent.code`). */
const seenKeyDown = new Set<string>();
/** Lỗi phát hiện ngay trong lúc ghi (chưa lưu vào config), ưu tiên hơn lỗi từ Rust. */
const recordErrors: Partial<Record<HotkeyName, string>> = {};

function render(s: SettingsState) {
  current = s;
  const c = s.config;
  saveDir.textContent = c.saveDir;
  if (document.activeElement !== pattern) pattern.value = c.fileNamePattern;
  preview.textContent = s.fileNamePreview;
  copyToClipboard.checked = c.copyToClipboard;
  afterCaptureNormal.value = c.afterCaptureNormal;
  stackEdge.value = c.stack.edge;
  stackMonitor.value = c.stack.monitor;
  launchAtStartup.checked = c.launchAtStartup;

  // Không loại được cửa sổ khỏi ảnh chụp -> app luôn ẩn UI khi chụp.
  hideUi.checked = c.hideUiDuringCapture || !s.captureExclusionOk;
  hideUi.disabled = !s.captureExclusionOk;
  exclusionNote.textContent = s.captureExclusionOk
    ? "Thường không cần bật: Windows đã loại toolbar và stack khỏi ảnh chụp. Bật nếu vẫn thấy chúng lọt vào ảnh."
    : "Windows này không hỗ trợ loại cửa sổ khỏi ảnh chụp (cần Windows 10 bản 2004 trở lên), nên Chup luôn ẩn toolbar và stack trong lúc chụp.";

  for (const input of hotkeyInputs) {
    const name = input.dataset.name as HotkeyName;
    if (input !== recording) input.value = c.hotkeys[name] || "Không dùng";
    const err = document.querySelector<HTMLElement>(`[data-error="${name}"]`)!;
    const msg = recordErrors[name] ?? s.hotkeyErrors[name] ?? "";
    err.textContent = msg;
    err.hidden = !msg;
  }
}

function setStatus(text: string, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
}

async function save(patch: DeepPartial<Config>) {
  try {
    render(await ipc.settingsUpdate(patch));
    setStatus("Đã lưu");
  } catch (e) {
    setStatus(errorMessage(e), true);
    if (current) render(current);
  }
}

// ---------- folder & tên file ----------

$<HTMLButtonElement>("pickDir").addEventListener("click", async () => {
  try {
    render(await ipc.pickSaveDir());
  } catch (e) {
    setStatus(errorMessage(e), true);
  }
});

$<HTMLButtonElement>("openDir").addEventListener("click", () => {
  ipc.openSaveDir().catch((e) => setStatus(errorMessage(e), true));
});

let patternTimer: number | undefined;
pattern.addEventListener("input", () => {
  window.clearTimeout(patternTimer);
  patternTimer = window.setTimeout(() => {
    if (pattern.value.trim()) void save({ fileNamePattern: pattern.value });
  }, 400);
});
pattern.addEventListener("blur", () => {
  if (!pattern.value.trim() && current) pattern.value = current.config.fileNamePattern;
});

// ---------- các lựa chọn khác ----------

copyToClipboard.addEventListener("change", () => void save({ copyToClipboard: copyToClipboard.checked }));
afterCaptureNormal.addEventListener("change", () =>
  void save({ afterCaptureNormal: afterCaptureNormal.value as Config["afterCaptureNormal"] }),
);
stackEdge.addEventListener("change", () => void save({ stack: { edge: stackEdge.value as Config["stack"]["edge"] } }));
stackMonitor.addEventListener("change", () =>
  void save({ stack: { monitor: stackMonitor.value as Config["stack"]["monitor"] } }),
);
launchAtStartup.addEventListener("change", () => void save({ launchAtStartup: launchAtStartup.checked }));
hideUi.addEventListener("change", () => void save({ hideUiDuringCapture: hideUi.checked }));

// ---------- ghi phím tắt ----------

/** Tên phím theo định dạng mà Rust (global-hotkey) parse được. */
function keyName(e: KeyboardEvent): string | null {
  const code = e.code;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/.test(code)) return null;
  return code || null;
}

function accelerator(e: KeyboardEvent): string | null {
  const key = keyName(e);
  if (!key) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(
    Boolean,
  ) as string[];
  const isFn = /^F\d+$/.test(key);
  // Không có Ctrl/Alt/Super thì dễ chặn phím gõ thường; chỉ cho phép với phím F.
  if (!isFn && !(e.ctrlKey || e.altKey || e.metaKey)) return null;
  return [...mods, key].join("+");
}

function stopRecording(input: HTMLInputElement) {
  input.classList.remove("recording");
  recording = null;
  if (current) render(current);
  ipc
    .hotkeysResume()
    .then(render)
    .catch(() => {});
}

for (const input of hotkeyInputs) {
  const name = input.dataset.name as HotkeyName;

  input.addEventListener("focus", () => {
    recording = input;
    seenKeyDown.clear();
    delete recordErrors[name];
    if (current) render(current);
    input.classList.add("recording");
    input.value = "Nhấn tổ hợp phím…";
    // Tạm tắt hotkey để tổ hợp cũ không kích hoạt chụp trong lúc ghi.
    void ipc.hotkeysPause();
  });

  input.addEventListener("blur", () => stopRecording(input));

  input.addEventListener("keydown", async (e) => {
    e.preventDefault();
    seenKeyDown.add(e.code);
    if (e.key === "Escape") return input.blur();
    if ((e.key === "Delete" || e.key === "Backspace") && !e.ctrlKey && !e.altKey && !e.metaKey) {
      delete recordErrors[name];
      await save({ hotkeys: { [name]: "" } });
      return input.blur();
    }
    const accel = accelerator(e);
    if (!accel) {
      const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
      input.value = mods.length ? `${mods.join("+")}+…` : "Cần có Ctrl, Alt hoặc dùng phím F1–F24";
      return;
    }
    input.value = accel;
    delete recordErrors[name];
    await save({ hotkeys: { [name]: accel } });
    input.blur();
  });

  // Tổ hợp đã bị app khác đăng ký (RegisterHotKey) thì Windows nuốt keydown của phím chính,
  // nhưng keyup vẫn tới, lúc modifier còn đang giữ. Thấy keyup mà chưa từng thấy keydown
  // => tổ hợp đã có chủ. Không lưu, giữ phím tắt cũ và báo ngay.
  // (Hotkey của chính Chup đã được tạm tắt khi bắt đầu ghi, nên không tự báo nhầm.)
  input.addEventListener("keyup", (e) => {
    if (recording !== input || seenKeyDown.has(e.code)) return;
    const accel = accelerator(e);
    if (!accel) return;
    e.preventDefault();
    recordErrors[name] =
      `${accel} đang được app khác (hoặc Windows) dùng nên Chup không đăng ký được. Hãy chọn tổ hợp khác.`;
    input.value = "Nhấn tổ hợp phím…";
    if (current) render(current);
  });
}

// ---------- khởi tạo ----------

void on("config:changed", render);
ipc
  .settingsGet()
  .then(render)
  .catch((e) => setStatus(errorMessage(e), true));

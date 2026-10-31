// Wrapper có kiểu cho command và event (SPEC 6.7).

import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { Config, DeepPartial, FrameState, SettingsState, StackState, Toast } from "./types";

export const ipc = {
  // overlay
  regionSelected: (r: { x: number; y: number; w: number; h: number }) => invoke<void>("region_selected", r),
  selectionCancelled: () => invoke<void>("selection_cancelled"),

  // frame / toolbar
  frameState: () => invoke<FrameState>("frame_state"),
  captureNow: () => invoke<void>("capture_now"),
  toggleFade: () => invoke<void>("toggle_fade"),
  closeRegion: () => invoke<void>("close_region"),

  // stack
  stackState: () => invoke<StackState>("stack_state"),
  stackReportSize: (width: number, height: number) => invoke<void>("stack_report_size", { width, height }),
  stackRemove: (id: string) => invoke<void>("stack_remove", { id }),
  stackClear: () => invoke<void>("stack_clear"),
  shotExists: (id: string) => invoke<boolean>("shot_exists", { id }),
  shotOpen: (id: string) => invoke<void>("shot_open", { id }),
  shotReveal: (id: string) => invoke<void>("shot_reveal", { id }),
  shotCopy: (id: string) => invoke<void>("shot_copy", { id }),

  // settings
  settingsGet: () => invoke<SettingsState>("settings_get"),
  settingsUpdate: (patch: DeepPartial<Config>) => invoke<SettingsState>("settings_update", { patch }),
  pickSaveDir: () => invoke<SettingsState>("pick_save_dir"),
  openSaveDir: () => invoke<void>("open_save_dir"),
  hotkeysPause: () => invoke<void>("hotkeys_pause"),
  hotkeysResume: () => invoke<SettingsState>("hotkeys_resume"),
};

interface Events {
  "selection:start": { mode: "live" | "freeze" };
  "frame:state": FrameState;
  "stack:changed": StackState;
  toast: Toast;
  "config:changed": SettingsState;
}

/** Nghe event Rust gửi tới riêng cửa sổ này (`emit_to(label, ...)`). */
export function on<K extends keyof Events>(event: K, cb: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return getCurrentWebviewWindow().listen<Events[K]>(event, (e) => cb(e.payload));
}

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

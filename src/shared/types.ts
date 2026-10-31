// Kiểu dữ liệu khớp với DTO bên Rust (serde camelCase).

export type FrameMode = "hidden" | "normal" | "faded";

export interface FrameState {
  mode: FrameMode;
  /** Lề quanh vùng chọn trong cửa sổ frame, CSS px. */
  marginCss: number;
  captureHotkey: string;
}

export interface Shot {
  id: string;
  path: string;
  thumbPath: string;
  thumbDataUrl: string;
  width: number;
  height: number;
  createdAt: string;
}

export type Edge = "left" | "right";

export interface StackState {
  shots: Shot[];
  edge: Edge;
}

export interface Toast {
  kind: "error" | "info";
  message: string;
}

export interface Config {
  version: number;
  saveDir: string;
  fileNamePattern: string;
  copyToClipboard: boolean;
  hotkeys: { selectRegion: string; capture: string };
  stack: { edge: Edge; monitor: "primary" | "cursor" };
  afterCaptureNormal: "close" | "keep";
  launchAtStartup: boolean;
  hideUiDuringCapture: boolean;
}

export type HotkeyName = keyof Config["hotkeys"];

export interface SettingsState {
  config: Config;
  hotkeyErrors: Partial<Record<HotkeyName, string>>;
  defaultSaveDir: string;
  fileNamePreview: string;
  captureExclusionOk: boolean;
}

/** Patch sâu: mọi field đều tuỳ chọn. */
export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

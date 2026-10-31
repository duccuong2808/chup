// Toolbar cạnh vùng chọn: Chụp / Mờ / Huỷ (SPEC 5.1, 5.2).
// Cửa sổ không lấy focus nên click vào đây không làm app đang dùng mất focus.

import "./style.scss";
import { ipc, on } from "../../shared/ipc";
import { icons } from "../../shared/icons";
import type { FrameState } from "../../shared/types";

const captureBtn = document.getElementById("capture") as HTMLButtonElement;
const fadeBtn = document.getElementById("fade") as HTMLButtonElement;
const closeBtn = document.getElementById("close") as HTMLButtonElement;

captureBtn.querySelector(".ic")!.innerHTML = icons.camera;
fadeBtn.querySelector(".ic")!.innerHTML = icons.fade;
closeBtn.querySelector(".ic")!.innerHTML = icons.close;

function apply(s: FrameState) {
  const faded = s.mode === "faded";
  document.body.classList.toggle("faded", faded);
  fadeBtn.classList.toggle("on", faded);
  captureBtn.title = s.captureHotkey ? `Chụp (${s.captureHotkey})` : "Chụp";
  fadeBtn.title = faded
    ? "Bỏ mờ: chỉnh lại vùng chọn"
    : "Làm mờ: giữ vùng chọn, click xuyên qua để dùng app khác";
}

captureBtn.addEventListener("click", async () => {
  captureBtn.disabled = true;
  try {
    await ipc.captureNow();
  } finally {
    captureBtn.disabled = false;
  }
});
fadeBtn.addEventListener("click", () => void ipc.toggleFade());
closeBtn.addEventListener("click", () => void ipc.closeRegion());
window.addEventListener("contextmenu", (e) => e.preventDefault());

void on("frame:state", apply);
void ipc.frameState().then(apply);

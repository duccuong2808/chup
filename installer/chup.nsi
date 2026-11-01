; Installer NSIS cho Chup. Cài cho user hiện tại, không cần quyền admin, không cần WebView2.
; Build: makensis /DVERSION=0.2.0 /DEXE=path\chup.exe /DOCR_DIR=path\ocr-dist /DOUTFILE=Chup_0.2.0_x64-setup.exe installer\chup.nsi
; OCR_DIR (artifact `chup-ocr` của CI) chứa chup-ocr.exe + NOTICE-ocr.txt. Không có thì bộ cài không có OCR tiếng Việt.
; Dữ liệu tiếng Việt không nằm trong bộ cài: mục "OCR tiếng Việt" (mặc định bỏ trống) chạy `chup.exe --install-ocr`
; để tải từ GitHub (tesseract-ocr/tessdata_fast).

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef EXE
  !define EXE "..\target\release\chup.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "Chup_${VERSION}_x64-setup.exe"
!endif

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Chup"

Name "Chup"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Chup"
BrandingText "Chup ${VERSION}"

!include "MUI2.nsh"
!define MUI_ICON "..\crates\app\icons\icon.ico"
!define MUI_UNICON "..\crates\app\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\chup.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Chạy Chup"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Vietnamese"

Section "Chup" SecChup
  SectionIn RO
  ; Đóng bản đang chạy để ghi đè được file exe (cả exe phụ OCR: nó chết theo Chup nhưng không tức thì).
  nsExec::Exec 'taskkill /IM chup.exe /F'
  nsExec::Exec 'taskkill /IM chup-ocr.exe /F'
  Sleep 300
  ; Bản cũ (Tauri) để lại dữ liệu WebView2; bản native không cần nữa.
  RMDir /r "$LOCALAPPDATA\app.chup.desktop\EBWebView"
  SetOutPath "$INSTDIR"
  File "/oname=chup.exe" "${EXE}"
!ifdef OCR_DIR
  File "${OCR_DIR}\chup-ocr.exe"
  File "${OCR_DIR}\NOTICE-ocr.txt"
!endif
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\Chup.lnk" "$INSTDIR\chup.exe"
  CreateShortcut "$DESKTOP\Chup.lnk" "$INSTDIR\chup.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Chup"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Chup"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\chup.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

!ifdef OCR_DIR
; Mặc định bỏ trống (/o): ai cần OCR tiếng Việt mới tải. Không tích vẫn tải được sau trong Cài đặt của Chup.
Section /o "OCR tiếng Việt" SecOcr
  DetailPrint "Đang tải gói OCR tiếng Việt…"
  ExecWait '"$INSTDIR\chup.exe" --install-ocr' $0
  ; 0 = xong, 2 = người dùng huỷ, khác = lỗi (chup.exe đã hỏi Thử lại).
  StrCmp $0 "0" ocr_done
  StrCmp $0 "2" ocr_done
  MessageBox MB_OK|MB_ICONEXCLAMATION "Chưa tải được gói OCR tiếng Việt. Có thể tải lại trong Cài đặt của Chup."
  ocr_done:
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecChup} "Chương trình Chup."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecOcr} "Tải thêm ~4,4 MB dữ liệu (GitHub tesseract-ocr/tessdata_fast) để copy chữ tiếng Việt từ ảnh. Tiếng Anh dùng OCR có sẵn của Windows, không cần tải."
!insertmacro MUI_FUNCTION_DESCRIPTION_END
!endif

Section "Uninstall"
  nsExec::Exec 'taskkill /IM chup.exe /F'
  nsExec::Exec 'taskkill /IM chup-ocr.exe /F'
  Sleep 300
  Delete "$INSTDIR\chup.exe"
  Delete "$INSTDIR\chup-ocr.exe"
  Delete "$INSTDIR\NOTICE-ocr.txt"
  Delete "$INSTDIR\uninstall.exe"
  ; Gói dữ liệu OCR tiếng Việt đã tải.
  RMDir /r "$LOCALAPPDATA\app.chup.desktop\ocr"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Chup.lnk"
  Delete "$DESKTOP\Chup.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Chup"
SectionEnd

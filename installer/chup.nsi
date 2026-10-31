; Installer NSIS cho Chup. Cài cho user hiện tại, không cần quyền admin, không cần WebView2.
; Build: makensis /DVERSION=0.2.0 /DEXE=path\chup.exe /DOUTFILE=Chup_0.2.0_x64-setup.exe installer\chup.nsi

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
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Vietnamese"

Section "Chup"
  ; Đóng bản đang chạy để ghi đè được file exe.
  nsExec::Exec 'taskkill /IM chup.exe /F'
  Sleep 300
  SetOutPath "$INSTDIR"
  File "/oname=chup.exe" "${EXE}"
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

Section "Uninstall"
  nsExec::Exec 'taskkill /IM chup.exe /F'
  Sleep 300
  Delete "$INSTDIR\chup.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Chup.lnk"
  Delete "$DESKTOP\Chup.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Chup"
SectionEnd

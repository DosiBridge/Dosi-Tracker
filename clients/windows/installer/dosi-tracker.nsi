; Dosi-Tracker Windows agent installer (NSIS).
;
; Deliberately a PER-USER install (no administrator rights required):
;   • installs to %LOCALAPPDATA%\Programs\DosiTracker
;   • autostart uses HKCU\...\Run, matching src/autostart.rs
;   • the agent must run in the signed-in user's session to capture screen/input,
;     so a machine-wide service would not work anyway.
;
; Build:  makensis /DVERSION=0.1.0 dosi-tracker.nsi
; Expects the release binary at ..\target\release\dosi-tracker.exe

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!define APP_NAME    "Dosi Tracker"
!define COMPANY     "DosiBridge"
!define EXE_NAME    "dosi-tracker.exe"
!define REG_UNINST  "Software\Microsoft\Windows\CurrentVersion\Uninstall\DosiTracker"
!define REG_RUN     "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${APP_NAME}"
OutFile "dosi-tracker-setup-${VERSION}.exe"
Unicode True
RequestExecutionLevel user               ; per-user: never prompts for UAC
InstallDir "$LOCALAPPDATA\Programs\DosiTracker"
InstallDirRegKey HKCU "Software\DosiTracker" "InstallDir"
ShowInstDetails show
ShowUnInstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName"    "${APP_NAME}"
VIAddVersionKey "CompanyName"    "${COMPANY}"
VIAddVersionKey "FileDescription" "${APP_NAME} activity agent"
VIAddVersionKey "FileVersion"    "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT"

!include "MUI2.nsh"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
; Offer to launch straight after install so the user can sign in immediately.
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE_NAME}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP_NAME} now"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; Stop a running instance so the .exe is not locked during install/uninstall.
!macro StopRunningAgent
  DetailPrint "Closing any running ${APP_NAME}…"
  nsExec::ExecToLog 'taskkill /F /IM "${EXE_NAME}"'
  Pop $0
  Sleep 800
!macroend

Section "Install"
  !insertmacro StopRunningAgent

  SetOutPath "$INSTDIR"
  File "..\target\release\${EXE_NAME}"

  WriteRegStr HKCU "Software\DosiTracker" "InstallDir" "$INSTDIR"

  ; Start Menu shortcut
  CreateDirectory "$SMPROGRAMS\${COMPANY}"
  CreateShortCut "$SMPROGRAMS\${COMPANY}\${APP_NAME}.lnk" "$INSTDIR\${EXE_NAME}"

  ; Start with Windows (the in-app Settings toggle manages the same value).
  WriteRegStr HKCU "${REG_RUN}" "DosiTracker" '"$INSTDIR\${EXE_NAME}"'

  ; Add/Remove Programs entry
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${REG_UNINST}" "Publisher"       "${COMPANY}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayIcon"     "$INSTDIR\${EXE_NAME}"
  WriteRegStr   HKCU "${REG_UNINST}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegDWORD HKCU "${REG_UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${REG_UNINST}" "NoRepair" 1

  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  !insertmacro StopRunningAgent

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"

  Delete "$SMPROGRAMS\${COMPANY}\${APP_NAME}.lnk"
  RMDir  "$SMPROGRAMS\${COMPANY}"

  DeleteRegValue HKCU "${REG_RUN}" "DosiTracker"
  DeleteRegKey   HKCU "${REG_UNINST}"
  DeleteRegKey   HKCU "Software\DosiTracker"

  ; Local queue + DPAPI-encrypted credentials live in %APPDATA%\DosiTracker.
  ; Removed on uninstall so no sign-in or unsynced capture data is left behind.
  RMDir /r "$APPDATA\DosiTracker"
SectionEnd

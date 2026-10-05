; Dosi-Tracker Windows agent installer (NSIS).
;
; Deliberately a PER-USER install (no administrator rights required):
;   • installs to %LOCALAPPDATA%\Programs\DosiTracker
;   • autostart uses HKCU\...\Run, matching src/autostart.rs
;   • the agent must run in the signed-in user's session to capture screen/input,
;     so a machine-wide service would not work anyway.
;
; Build:  makensis /DVERSION=0.1.0 dosi-tracker.nsi
;         (CI passes the version from Cargo.toml; VERSION_NUMERIC is the same
;         without a pre-release suffix, for the Windows version resource.)
; Expects the release binary at ..\target\release\dosi-tracker.exe
;
; Command line (also with /S for a silent install):
;   /SERVER=<url>      the server address the agent signs in to; written to
;                      config.toml next to the program (api_base_url)
;   /DASHBOARD=<url>   the web dashboard opened from the app (dashboard_url)
; Other settings already in that config.toml are kept.
;
; Uninstaller:
;   /KEEPDATA          keep the sign-in, preferences, logs and offline queue
;                      (otherwise it asks; a silent uninstall keeps them, so
;                      tracked time not uploaded yet is never lost)

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!ifndef VERSION_NUMERIC
  !define VERSION_NUMERIC "${VERSION}"
!endif

!define APP_NAME    "Dosi Tracker"
!define COMPANY     "DosiBridge"
!define EXE_NAME    "dosi-tracker.exe"
!define CONFIG_FILE "config.toml"
!define DATA_DIR    "DosiTracker"
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

VIProductVersion "${VERSION_NUMERIC}.0"
VIAddVersionKey "ProductName"    "${APP_NAME}"
VIAddVersionKey "CompanyName"    "${COMPANY}"
VIAddVersionKey "FileDescription" "${APP_NAME} activity agent"
VIAddVersionKey "FileVersion"    "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT"

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

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

; Addresses from the command line (empty when not given).
Var ServerUrl
Var DashboardUrl
; What SetConfigValue writes.
Var CfgKey
Var CfgValue

; Ask a running agent to quit gracefully — it saves the block in progress
; first — using EXE (a build that knows --quit; it waits up to 30 s), then
; end whatever still runs: an older build that does not listen for --quit,
; or one that did not finish in time. The file must not be locked when it is
; replaced or removed.
!macro StopRunningAgent EXE
  DetailPrint "Asking ${APP_NAME} to save its work and close…"
  ${If} ${FileExists} "${EXE}"
    nsExec::ExecToLog '"${EXE}" --quit'
    Pop $0
  ${EndIf}
  nsExec::ExecToLog 'taskkill /F /IM "${EXE_NAME}"'
  Pop $0
  Sleep 800
!macroend

; The value of an option such as /SERVER= on the command line, or "" when it
; is not there. GetOptions would cut an address at its first "/" (it takes
; "/" for the start of the next option), so the value here runs to the next
; blank or quote, and may itself be quoted: /SERVER="https://…".
; In: the command line, then the option (pushed in that order). Out: the value.
Function GetArgValue
  Exch $1
  Exch
  Exch $0
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  StrCpy $6 ""
  StrLen $3 $0
  StrLen $4 $1
  StrCpy $2 0
  find_arg:
    ${If} $2 >= $3
      Goto arg_done
    ${EndIf}
    StrCpy $5 $0 $4 $2
    ${If} $5 == $1
      ; Only where an argument starts.
      ${If} $2 = 0
        Goto arg_found
      ${EndIf}
      IntOp $5 $2 - 1
      StrCpy $5 $0 1 $5
      ${If} $5 == " "
      ${OrIf} $5 == '"'
        Goto arg_found
      ${EndIf}
    ${EndIf}
    IntOp $2 $2 + 1
    Goto find_arg
  arg_found:
    IntOp $2 $2 + $4
    StrCpy $5 $0 1 $2
    ${If} $5 == '"'
      IntOp $2 $2 + 1
    ${EndIf}
    take_char:
      StrCpy $5 $0 1 $2
      ${If} $5 == ""
      ${OrIf} $5 == " "
      ${OrIf} $5 == '"'
        Goto arg_done
      ${EndIf}
      StrCpy $6 "$6$5"
      IntOp $2 $2 + 1
      Goto take_char
  arg_done:
  StrCpy $0 $6
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Exch
  Pop $1
  Exch $0
FunctionEnd

; A server address the agent can use: http:// or https://, a host after it,
; and no quotes, backslashes or blanks (it is written into a TOML string).
; In: the address on the stack. Out: "1" or "0" on the stack.
Function ValidateUrl
  Exch $0
  Push $1
  Push $2
  Push $3
  StrCpy $1 0
  StrCpy $2 $0 8
  StrCpy $3 $0 7
  ${If} $2 == "https://"
    StrLen $2 $0
    ${If} $2 > 8
      StrCpy $1 1
    ${EndIf}
  ${ElseIf} $3 == "http://"
    StrLen $2 $0
    ${If} $2 > 7
      StrCpy $1 1
    ${EndIf}
  ${EndIf}
  StrCpy $2 0
  scan:
    StrCpy $3 $0 1 $2
    StrCmp $3 "" scanned
    ${If} $3 == '"'
    ${OrIf} $3 == "\"
    ${OrIf} $3 == "'"
    ${OrIf} $3 == " "
    ${OrIf} $3 == "$\t"
      StrCpy $1 0
    ${EndIf}
    IntOp $2 $2 + 1
    Goto scan
  scanned:
  StrCpy $0 $1
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Strip leading spaces and tabs. In and out on the stack.
Function TrimLeadingBlanks
  Exch $0
  Push $1
  trim:
    StrCpy $1 $0 1
    ${If} $1 == " "
    ${OrIf} $1 == "$\t"
      StrCpy $0 $0 "" 1
      Goto trim
    ${EndIf}
  Pop $1
  Exch $0
FunctionEnd

; Set $CfgKey to the string $CfgValue in config.toml next to the program.
; Lines that already set the key are dropped and `key = "value"` is written
; first (top-level keys come before any [table]); every other byte of the
; file is kept as it was, whatever its encoding.
Function SetConfigValue
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9
  Push $R0
  Push $R1
  Push $R2

  StrCpy $0 "$INSTDIR\${CONFIG_FILE}"
  StrLen $8 $CfgKey
  ; One character per line of the old file: 1 = it sets the key, drop it.
  StrCpy $6 ""

  ${If} ${FileExists} $0
    FileOpen $2 $0 r
    ; A UTF-8 byte-order mark is not part of the first line.
    ClearErrors
    FileReadByte $2 $4
    FileReadByte $2 $R0
    FileReadByte $2 $R1
    ${If} ${Errors}
    ${OrIf} $4 <> 239
    ${OrIf} $R0 <> 187
    ${OrIf} $R1 <> 191
      FileSeek $2 0 SET
    ${EndIf}
    ; Pass 1: which lines set the key? (Only ASCII is compared, so the
    ; code page FileRead decodes with does not matter.)
    StrCpy $9 1
    find_line:
      ClearErrors
      FileRead $2 $4
      IfErrors find_done
      ${If} $9 = 1
        Push $4
        Call TrimLeadingBlanks
        Pop $7
        StrCpy $R0 $7 $8
        StrCpy $R1 $7 1 $8
        StrCpy $R2 0
        ${If} $R0 == $CfgKey
          ${If} $R1 == " "
          ${OrIf} $R1 == "$\t"
          ${OrIf} $R1 == "="
            StrCpy $R2 1
          ${EndIf}
        ${EndIf}
        StrCpy $6 "$6$R2"
      ${EndIf}
      ; A long line arrives in pieces: only one ending in a newline ends it.
      StrCpy $7 $4 1 -1
      ${If} $7 == "$\n"
        StrCpy $9 1
      ${Else}
        StrCpy $9 0
      ${EndIf}
      Goto find_line
    find_done:
    FileClose $2
  ${EndIf}

  GetTempFileName $1 $INSTDIR
  FileOpen $3 $1 w
  ${If} ${FileExists} $0
    FileOpen $2 $0 r
    ClearErrors
    FileReadByte $2 $4
    FileReadByte $2 $R0
    FileReadByte $2 $R1
    ${If} ${Errors}
    ${OrIf} $4 <> 239
    ${OrIf} $R0 <> 187
    ${OrIf} $R1 <> 191
      FileSeek $2 0 SET
    ${Else}
      ; The byte-order mark stays first.
      FileWriteByte $3 239
      FileWriteByte $3 187
      FileWriteByte $3 191
    ${EndIf}
    FileWrite $3 '$CfgKey = "$CfgValue"$\r$\n'
    ; Pass 2: copy every byte of the lines that are kept.
    StrCpy $5 0
    StrCpy $R2 $6 1 0
    copy_byte:
      ClearErrors
      FileReadByte $2 $4
      IfErrors copy_done
      ${If} $R2 != 1
        FileWriteByte $3 $4
      ${EndIf}
      ${If} $4 = 10
        IntOp $5 $5 + 1
        StrCpy $R2 $6 1 $5
      ${EndIf}
      Goto copy_byte
    copy_done:
    FileClose $2
  ${Else}
    FileWrite $3 '$CfgKey = "$CfgValue"$\r$\n'
  ${EndIf}
  FileClose $3
  ClearErrors
  Delete $0
  Rename $1 $0
  ${If} ${Errors}
    ; The old file stays as it was; say so rather than lose it.
    Delete $1
    DetailPrint "Could not update $0."
    MessageBox MB_ICONEXCLAMATION "Could not set $CfgKey in $0. Set it there by hand, or enter the address when signing in." /SD IDOK
  ${EndIf}

  Pop $R2
  Pop $R1
  Pop $R0
  Pop $9
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

Function .onInit
  ${GetParameters} $R0
  Push $R0
  Push "/SERVER="
  Call GetArgValue
  Pop $ServerUrl
  Push $R0
  Push "/DASHBOARD="
  Call GetArgValue
  Pop $DashboardUrl

  ${If} $ServerUrl != ""
    Push $ServerUrl
    Call ValidateUrl
    Pop $R1
    ${If} $R1 != 1
      MessageBox MB_ICONSTOP "/SERVER must be a full address such as https://tracker.example.com (got: $ServerUrl)." /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
  ${If} $DashboardUrl != ""
    Push $DashboardUrl
    Call ValidateUrl
    Pop $R1
    ${If} $R1 != 1
      MessageBox MB_ICONSTOP "/DASHBOARD must be a full address such as https://app.example.com (got: $DashboardUrl)." /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "Install"
  ; The new build first, in a temporary folder: it knows how to ask the
  ; running agent to quit gracefully (an older one would only be ended).
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "..\target\release\${EXE_NAME}"
  !insertmacro StopRunningAgent "$PLUGINSDIR\${EXE_NAME}"

  SetOutPath "$INSTDIR"
  ClearErrors
  CopyFiles /SILENT "$PLUGINSDIR\${EXE_NAME}" "$INSTDIR\${EXE_NAME}"
  ${If} ${Errors}
    Abort "Could not write $INSTDIR\${EXE_NAME}. Close ${APP_NAME} and run the installer again."
  ${EndIf}

  ; Where the agent signs in (and the dashboard it links to), for every
  ; user of this install; the address typed at sign-in still wins.
  ${If} $ServerUrl != ""
    DetailPrint "Server address: $ServerUrl"
    StrCpy $CfgKey "api_base_url"
    StrCpy $CfgValue $ServerUrl
    Call SetConfigValue
  ${EndIf}
  ${If} $DashboardUrl != ""
    DetailPrint "Web dashboard: $DashboardUrl"
    StrCpy $CfgKey "dashboard_url"
    StrCpy $CfgValue $DashboardUrl
    Call SetConfigValue
  ${EndIf}

  WriteRegStr HKCU "Software\DosiTracker" "InstallDir" "$INSTDIR"

  ; Start Menu shortcut
  CreateDirectory "$SMPROGRAMS\${COMPANY}"
  CreateShortCut "$SMPROGRAMS\${COMPANY}\${APP_NAME}.lnk" "$INSTDIR\${EXE_NAME}"

  ; Start with Windows (the in-app Settings toggle manages the same value).
  ; --autostart lets the app start quietly in the tray at sign-in.
  WriteRegStr HKCU "${REG_RUN}" "DosiTracker" '"$INSTDIR\${EXE_NAME}" --autostart'

  ; Add/Remove Programs entry
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${REG_UNINST}" "Publisher"       "${COMPANY}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayIcon"     "$INSTDIR\${EXE_NAME}"
  WriteRegStr   HKCU "${REG_UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr   HKCU "${REG_UNINST}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${REG_UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${REG_UNINST}" "NoRepair" 1

  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; The installed build knows --quit: the block in progress is saved first.
  !insertmacro StopRunningAgent "$INSTDIR\${EXE_NAME}"

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\${CONFIG_FILE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"

  Delete "$SMPROGRAMS\${COMPANY}\${APP_NAME}.lnk"
  RMDir  "$SMPROGRAMS\${COMPANY}"

  DeleteRegValue HKCU "${REG_RUN}" "DosiTracker"
  DeleteRegKey   HKCU "${REG_UNINST}"
  DeleteRegKey   HKCU "Software\DosiTracker"

  ; The user's data: the DPAPI-encrypted sign-in and the preferences
  ; (%APPDATA%\DosiTracker), and the offline queue — with any tracked time
  ; not uploaded yet — the logs and the device id (%LOCALAPPDATA%\DosiTracker).
  ${un.GetParameters} $R0
  ClearErrors
  ${un.GetOptions} $R0 "/KEEPDATA" $R1
  ${IfNot} ${Errors}
    DetailPrint "Keeping your ${APP_NAME} data (/KEEPDATA)."
  ${Else}
    MessageBox MB_YESNO|MB_ICONQUESTION "Also delete your ${APP_NAME} data from this computer?$\r$\n$\r$\nThis removes your saved sign-in and preferences, the logs, and any tracked time that has not been uploaded yet.$\r$\n$\r$\nChoose No to keep it for a later install." /SD IDNO IDNO keep_data
      RMDir /r "$APPDATA\${DATA_DIR}"
      RMDir /r "$LOCALAPPDATA\${DATA_DIR}"
      DetailPrint "Removed your ${APP_NAME} data."
      Goto data_done
    keep_data:
      DetailPrint "Kept your ${APP_NAME} data in $APPDATA\${DATA_DIR} and $LOCALAPPDATA\${DATA_DIR}."
    data_done:
  ${EndIf}
SectionEnd

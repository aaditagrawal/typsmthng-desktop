!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinMessages.nsh"

Name "typsmthng"
OutFile "${OUTPUT_DIR}\${OUTPUT_NAME}"
InstallDir "$LOCALAPPDATA\typsmthng"
InstallDirRegKey HKCU "Software\typsmthng" "InstallDir"
RequestExecutionLevel user

; Pages
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"

  ; Copy application files
  File /r "${BUILD_DIR}\*.*"

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Start menu shortcut
  CreateDirectory "$SMPROGRAMS\typsmthng"
  CreateShortcut "$SMPROGRAMS\typsmthng\typsmthng.lnk" "$INSTDIR\typsmthng.exe"
  CreateShortcut "$SMPROGRAMS\typsmthng\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  ; Registry entries for Add/Remove Programs
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" \
    "DisplayName" "typsmthng"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" \
    "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" \
    "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" \
    "Publisher" "typsmthng"
  WriteRegStr HKCU "Software\typsmthng" "InstallDir" "$INSTDIR"

  ; File association for .typ files
  WriteRegStr HKCU "Software\Classes\.typ" "" "typsmthng.typ"
  WriteRegStr HKCU "Software\Classes\typsmthng.typ" "" "Typst Document"
  WriteRegStr HKCU "Software\Classes\typsmthng.typ\shell\open\command" "" '"$INSTDIR\typsmthng.exe" "%1"'

  ; Add to user PATH
  ReadRegStr $0 HKCU "Environment" "Path"
  ${If} $0 != ""
    WriteRegExpandStr HKCU "Environment" "Path" "$0;$INSTDIR"
  ${Else}
    WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR"
  ${EndIf}
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

Section "Uninstall"
  ; Remove files
  RMDir /r "$INSTDIR"

  ; Remove shortcuts
  RMDir /r "$SMPROGRAMS\typsmthng"
  Delete "$DESKTOP\typsmthng.lnk"

  ; Remove file association
  DeleteRegKey HKCU "Software\Classes\.typ"
  DeleteRegKey HKCU "Software\Classes\typsmthng.typ"

  ; Remove registry entries
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng"
  DeleteRegKey HKCU "Software\typsmthng"

  ; Note: PATH cleanup is intentionally skipped to avoid corrupting PATH entries
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

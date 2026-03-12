!include "MUI2.nsh"

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
SectionEnd

Section "Uninstall"
  ; Remove files
  RMDir /r "$INSTDIR"

  ; Remove shortcuts
  RMDir /r "$SMPROGRAMS\typsmthng"
  Delete "$DESKTOP\typsmthng.lnk"

  ; Remove registry entries
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng"
  DeleteRegKey HKCU "Software\typsmthng"
SectionEnd

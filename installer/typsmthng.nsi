!include "MUI2.nsh"

Name "typsmthng"
OutFile "${OUTPUT_DIR}\${OUTPUT_NAME}"
InstallDir "$LOCALAPPDATA\typsmthng"
InstallDirRegKey HKCU "Software\typsmthng" "InstallDir"
RequestExecutionLevel user

!define MUI_ICON "${BUILD_DIR}\icon.ico"
!define MUI_UNICON "${BUILD_DIR}\icon.ico"

; Pages
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"

  ; Copy application files
  File /r "${BUILD_DIR}\typsmthng\*.*"

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Start menu shortcut
  CreateDirectory "$SMPROGRAMS\typsmthng"
  CreateShortcut "$SMPROGRAMS\typsmthng\typsmthng.lnk" "$INSTDIR\typsmthng.exe"
  CreateShortcut "$SMPROGRAMS\typsmthng\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  ; Desktop shortcut (optional, controlled by /DESKTOP flag)
  ${If} ${FileExists} "$DESKTOP"
    CreateShortcut "$DESKTOP\typsmthng.lnk" "$INSTDIR\typsmthng.exe"
  ${EndIf}

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

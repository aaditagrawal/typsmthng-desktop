Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinMessages.nsh"
!include "StrFunc.nsh"
${StrStr}
${StrCase}
${UnStrRep}
!include "..\..\build\windows-files.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif

Name "typsmthng"
OutFile "..\..\build\release\typsmthng-windows-x64.exe"
InstallDir "$LOCALAPPDATA\typsmthng"
InstallDirRegKey HKCU "Software\typsmthng" "InstallDir"
RequestExecutionLevel user
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  ; A directory chosen by the user can contain unrelated files. Never remove
  ; it recursively, on either installation or uninstallation.
  !insertmacro RemoveInstalledFiles
  SetOutPath "$INSTDIR"
  File /r "..\..\build\windows\*"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS\typsmthng"
  CreateShortcut "$SMPROGRAMS\typsmthng\typsmthng.lnk" "$INSTDIR\bin\typsmthng.exe"
  CreateShortcut "$SMPROGRAMS\typsmthng\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" "DisplayName" "typsmthng"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng" "Publisher" "typsmthng"
  WriteRegStr HKCU "Software\typsmthng" "InstallDir" "$INSTDIR"

  ReadRegStr $0 HKCU "Software\Classes\.typ" ""
  ${If} $0 != "typsmthng.typ"
    WriteRegStr HKCU "Software\typsmthng" "PrevTypAssoc" "$0"
  ${EndIf}
  WriteRegStr HKCU "Software\Classes\.typ" "" "typsmthng.typ"
  WriteRegStr HKCU "Software\Classes\typsmthng.typ" "" "Typst Document"
  WriteRegStr HKCU "Software\Classes\typsmthng.typ\shell\open\command" "" '"$INSTDIR\bin\typsmthng.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\Directory\shell\typsmthng" "" "Open with typsmthng"
  WriteRegStr HKCU "Software\Classes\Directory\shell\typsmthng\command" "" '"$INSTDIR\bin\typsmthng.exe" "%1"'

  ReadRegStr $0 HKCU "Environment" "Path"
  ${If} $0 == ""
    WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR\bin"
  ${Else}
    ${StrCase} $2 ";$0;" "L"
    ${StrCase} $3 "$INSTDIR\bin" "L"
    ${StrStr} $1 "$2" ";$3;"
    ${If} $1 == ""
      WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR\bin;$0"
    ${EndIf}
  ${EndIf}
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

Section "Uninstall"
  ; An older install must not unregister a newer copy at another location.
  ReadRegStr $9 HKCU "Software\typsmthng" "InstallDir"
  StrCmp $9 $INSTDIR 0 remove_payload
  ReadRegStr $0 HKCU "Software\Classes\.typ" ""
  ${If} $0 == "typsmthng.typ"
    ReadRegStr $1 HKCU "Software\typsmthng" "PrevTypAssoc"
    ${If} $1 != ""
      WriteRegStr HKCU "Software\Classes\.typ" "" "$1"
    ${Else}
      DeleteRegValue HKCU "Software\Classes\.typ" ""
    ${EndIf}
  ${EndIf}
  Delete "$SMPROGRAMS\typsmthng\typsmthng.lnk"
  Delete "$SMPROGRAMS\typsmthng\Uninstall.lnk"
  RMDir "$SMPROGRAMS\typsmthng"
  Delete "$SMPROGRAMS\typsmthng.lnk"
  DeleteRegKey HKCU "Software\Classes\typsmthng.typ"
  DeleteRegKey HKCU "Software\Classes\Directory\shell\typsmthng"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\typsmthng"
  ReadRegStr $0 HKCU "Environment" "Path"
  ${UnStrRep} $1 "$0" "$INSTDIR\bin;" ""
  ${UnStrRep} $1 "$1" ";$INSTDIR\bin" ""
  ${If} $1 == "$INSTDIR\bin"
    DeleteRegValue HKCU "Environment" "Path"
  ${Else}
    WriteRegExpandStr HKCU "Environment" "Path" "$1"
  ${EndIf}
  DeleteRegKey HKCU "Software\typsmthng"
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
remove_payload:
  !insertmacro RemoveInstalledFiles
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd

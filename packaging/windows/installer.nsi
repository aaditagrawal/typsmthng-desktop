Unicode true
!include "WinMessages.nsh"
Name "typsmthng"
OutFile "..\..\build\release\typsmthng-windows-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\typsmthng"
RequestExecutionLevel user
Page directory
Page instfiles

Section "Install"
  SetOutPath "$INSTDIR"
  File /r "..\..\build\windows\*"
  CreateShortcut "$SMPROGRAMS\typsmthng.lnk" "$INSTDIR\typsmthng.exe"
  ReadRegDWORD $1 HKCU "Software\typsmthng" "InstallInitialized"
  StrCmp $1 1 association_ready
    ReadRegStr $0 HKCU "Software\Classes\.typ" ""
    WriteRegStr HKCU "Software\typsmthng" "PreviousTypProgID" "$0"
    WriteRegDWORD HKCU "Software\typsmthng" "InstallInitialized" 1
  association_ready:
  WriteRegStr HKCU "Software\Classes\.typ" "" "typsmthng.typ"
  WriteRegStr HKCU "Software\Classes\typsmthng.typ\shell\open\command" "" '"$INSTDIR\typsmthng.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\Directory\shell\typsmthng" "" "Open with typsmthng"
  WriteRegStr HKCU "Software\Classes\Directory\shell\typsmthng\command" "" '"$INSTDIR\typsmthng.exe" "%1"'
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

Section "Uninstall"
  ReadRegStr $1 HKCU "Software\Classes\.typ" ""
  StrCmp $1 "typsmthng.typ" 0 association_done
    ReadRegStr $0 HKCU "Software\typsmthng" "PreviousTypProgID"
    StrCmp $0 "" remove_association
    StrCmp $0 "typsmthng.typ" remove_association
      WriteRegStr HKCU "Software\Classes\.typ" "" "$0"
      Goto association_done
    remove_association:
      DeleteRegValue HKCU "Software\Classes\.typ" ""
  association_done:
  RMDir /r "$INSTDIR"
  Delete "$SMPROGRAMS\typsmthng.lnk"
  DeleteRegKey HKCU "Software\Classes\typsmthng.typ"
  DeleteRegKey HKCU "Software\Classes\Directory\shell\typsmthng"
  DeleteRegKey HKCU "Software\typsmthng"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

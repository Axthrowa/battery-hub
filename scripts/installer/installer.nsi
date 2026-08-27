Unicode true
!include "MUI2.nsh"
!define APP        "Battery Hub"
!define PUBLISHER  "Axthrowa"
!define VERSION    "0.1.0"
!define EXE        "battery-hub.exe"
!define AUMID      "com.axthrowa.battery-hub"
!define REGKEY     "Software\Microsoft\Windows\CurrentVersion\Uninstall\Battery Hub"
Name "${APP}"
OutFile "Battery Hub_${VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\${APP}"
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "0.1.0.0"
VIAddVersionKey "ProductName"     "${APP}"
VIAddVersionKey "CompanyName"     "${PUBLISHER}"
VIAddVersionKey "LegalCopyright"  "Copyright (c) ${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "FileVersion"     "${VERSION}"
VIAddVersionKey "ProductVersion"  "${VERSION}"
!define MUI_ICON   "icon.ico"
!define MUI_UNICON "icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Battery Hub'i baslat"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "English"
!macro StopRunning
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  Pop $0
  Sleep 800
!macroend
Section "Install"
  !insertmacro StopRunning
  SetOutPath "$INSTDIR"
  ; Keep whatever was working before this runs. Smart App Control judges each
  ; file on its own: a build it refuses installs perfectly and then will not
  ; start, and without this the machine is left with no working copy at all.
  IfFileExists "$INSTDIR\${EXE}" 0 +2
    CopyFiles /SILENT "$INSTDIR\${EXE}" "$INSTDIR\onceki-${EXE}"
  File "${EXE}"
  File "WebView2Loader.dll"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortCut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
  ; Toasts are addressed by AppUserModelID; without this the shell drops them.
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${AUMID}" "DisplayName" "${APP}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${AUMID}" "IconUri" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${REGKEY}" "DisplayName"     "${APP}"
  WriteRegStr HKCU "${REGKEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr HKCU "${REGKEY}" "Publisher"       "${PUBLISHER}"
  WriteRegStr HKCU "${REGKEY}" "DisplayIcon"     "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${REGKEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${REGKEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${REGKEY}" "NoRepair" 1
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)'
SectionEnd
Section "Uninstall"
  !insertmacro StopRunning
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\onceki-${EXE}"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$SMPROGRAMS\${APP}.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${AUMID}"
  DeleteRegKey HKCU "${REGKEY}"
  DeleteRegKey HKCU "Software\${APP}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${APP}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)'
SectionEnd

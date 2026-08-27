Unicode true
!include "MUI2.nsh"
!define APP        "Battery Hub"
!define PUBLISHER  "Axthrowa"
!define VERSION    "0.2.0"
!define EXE        "battery-hub.exe"
!define AUMID      "com.axthrowa.battery-hub"
!define REGKEY     "Software\Microsoft\Windows\CurrentVersion\Uninstall\Battery Hub"
Name "${APP}"
OutFile "Battery Hub_${VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\${APP}"
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "0.2.0.0"
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

; Kept ASCII on purpose: the rest of this script is, and a mangled prompt is
; worse than a plain one.
LangString UninstData ${LANG_TURKISH} "Ayarlar, eklenen cihazlar, kart gorselleri ve bildirim sesleri de silinsin mi?$\r$\n$\r$\nHayir derseniz yeniden kurdugunuzda hepsi yerinde olur."
LangString UninstData ${LANG_ENGLISH} "Also delete settings, added devices, card images and notification sounds?$\r$\n$\r$\nChoosing No keeps them for the next install."
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
  ; Left behind by the PowerShell installer this one replaced.
  Delete "$INSTDIR\Kur.cmd"
  Delete "$INSTDIR\Kaldir.cmd"
  Delete "$INSTDIR\install.ps1"
  Delete "$INSTDIR\uninstall.ps1"
  Delete "$SMPROGRAMS\${APP}.lnk"

  ; The settings, the taught devices, the pictures and the sounds are the
  ; user's work, not the program's, and someone uninstalling to put a newer
  ; build in its place does not want to set all of it up again. So it is asked
  ; rather than assumed -- and in a silent uninstall, where nobody can be
  ; asked, it is kept.
  IfSilent KeepData
  MessageBox MB_YESNO|MB_ICONQUESTION "$(UninstData)" IDNO KeepData
    Delete "$INSTDIR\devices.json"
    Delete "$INSTDIR\notification-full.wav"
    Delete "$INSTDIR\notification-low.wav"
    RMDir /r "$APPDATA\com.axthrowa.battery-hub"
  KeepData:

  ; Diagnostics are the program's own scribble, not the user's, so they go
  ; either way -- and without this the folder can never be removed.
  Delete "$INSTDIR\diagnostics.log"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${AUMID}"
  DeleteRegKey HKCU "${REGKEY}"
  DeleteRegKey HKCU "Software\${APP}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${APP}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)'
SectionEnd

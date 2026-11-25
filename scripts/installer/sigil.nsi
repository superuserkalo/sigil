; Sigil's Windows installer. Built on macOS or Linux with makensis by scripts/package-installer.sh:
;   makensis -DVERSION=0.1.42 -DEXE=/path/Sigil.exe -DNOTICES=/path/THIRD_PARTY_NOTICES.md -DICON=/path/sigil.ico -DOUT=/path/Sigil-Setup.exe sigil.nsi
;
; Per user, no administrator rights: Sigil goes into %LOCALAPPDATA%\Programs\Sigil, where its own
; updater can replace Sigil.exe later. The installer writes nothing to the game and never touches
; Sigil's data (settings, backups, profiles): uninstalling leaves that folder alone.
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!include "MUI2.nsh"

Name "Sigil"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Sigil"
InstallDirRegKey HKCU "Software\Sigil" "InstallDir"
BrandingText "Sigil ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Sigil"
VIAddVersionKey "FileDescription" "Sigil Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Sigil contributors"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Sigil.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Sigil"

Section "Sigil"
  SetOutPath "$INSTDIR"
  ; A running Sigil holds its file: the default Abort, Retry, Ignore prompt covers that.
  File "/oname=Sigil.exe" "${EXE}"
  File "/oname=THIRD_PARTY_NOTICES.txt" "${NOTICES}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Sigil.lnk" "$INSTDIR\Sigil.exe"
  CreateShortcut "$DESKTOP\Sigil.lnk" "$INSTDIR\Sigil.exe"
  WriteRegStr HKCU "Software\Sigil" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Sigil"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Sigil"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\Sigil.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/superuserkalo/sigil"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\Sigil.exe"
  ; What Sigil's own updater may have left beside it.
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\Sigil.exe.old"
  RMDir /r "$INSTDIR\assets.old"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Sigil.lnk"
  Delete "$DESKTOP\Sigil.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\Sigil"
SectionEnd

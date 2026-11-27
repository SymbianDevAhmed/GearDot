Unicode true
Name "GearDot Setup"
OutFile "GearDot-Setup-x64.exe"
Icon "GearDot.ico"
RequestExecutionLevel user
SilentInstall silent
SetCompressor /SOLID lzma

!ifndef VERSION
!error "Pass the version with -DVERSION"
!endif
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "GearDot"
VIAddVersionKey "FileDescription" "GearDot Setup for Windows x64"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Ahmed Gaber"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 Ahmed Gaber"

Section
    StrCpy $0 "$WINDIR\Sysnative\msiexec.exe"
    IfFileExists "$0" +4
    MessageBox MB_ICONSTOP "GearDot requires 64-bit Windows."
    SetErrorLevel 1633
    Quit

    InitPluginsDir
    SetOutPath "$PLUGINSDIR"
    File "GearDot-Setup-x64.msi"
    ClearErrors
    ExecWait '"$0" /i "$PLUGINSDIR\GearDot-Setup-x64.msi"' $1
    IfErrors 0 +4
    MessageBox MB_ICONSTOP "Windows Installer could not be started."
    SetErrorLevel 1
    Quit

    SetErrorLevel $1
SectionEnd

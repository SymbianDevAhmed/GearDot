Unicode true
Name "GearDot"
Caption "GearDot Setup"
OutFile "GearDot-Setup-x64.exe"
Icon "GearDot.ico"
RequestExecutionLevel user
SetCompressor /SOLID lzma
InstallDir "$LOCALAPPDATA\Programs\GearDot"
ShowInstDetails show

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"

!ifndef VERSION
!error "Pass -DVERSION"
!endif
!ifndef PRODUCT_CODE
!error "Pass the embedded MSI product code with -DPRODUCT_CODE"
!endif

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "GearDot"
VIAddVersionKey "FileDescription" "GearDot Setup for Windows x64"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Ahmed Gaber"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 Ahmed Gaber"

Var Mode
Var RepairChoice
Var Installer
Var LogFile
Var Result

!define MUI_ICON "GearDot.ico"
!define MUI_ABORTWARNING
!define MUI_PAGE_CUSTOMFUNCTION_PRE NewInstallOnly
!insertmacro MUI_PAGE_WELCOME
Page custom MaintenancePage MaintenanceLeave
!define MUI_PAGE_CUSTOMFUNCTION_PRE NewInstallOnly
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE LaunchPageOnly
!define MUI_FINISHPAGE_RUN "$INSTDIR\GearDot.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch GearDot"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    SetShellVarContext current
    SetRegView 64
    StrCpy $Installer "$WINDIR\Sysnative\msiexec.exe"
    ${IfNot} ${FileExists} "$Installer"
        MessageBox MB_ICONSTOP "GearDot requires 64-bit Windows." /SD IDOK
        SetErrorLevel 1633
        Abort
    ${EndIf}

    ReadRegStr $0 HKCU "Software\GearDot" "InstallFolder"
    ${If} $0 != ""
        StrCpy $INSTDIR $0
    ${EndIf}
    StrCpy $Mode "install"
    System::Call 'msi::MsiQueryProductStateW(w "${PRODUCT_CODE}") i.r0'
    ${If} $0 == 5
        StrCpy $Mode "repair"
    ${EndIf}
FunctionEnd

Function NewInstallOnly
    ${If} $Mode != "install"
        Abort
    ${EndIf}
FunctionEnd

Function LaunchPageOnly
    ${If} $Mode != "install"
    ${OrIf} $Result == 3010
        Abort
    ${EndIf}
FunctionEnd

Function MaintenancePage
    ${If} $Mode == "install"
        Abort
    ${EndIf}
    !insertmacro MUI_HEADER_TEXT "Maintain GearDot" "Choose an action and click Next."
    nsDialogs::Create 1018
    Pop $0
    ${If} $0 == error
        Abort
    ${EndIf}
    ${NSD_CreateRadioButton} 0 12u 100% 12u "&Repair GearDot"
    Pop $RepairChoice
    ${NSD_CreateLabel} 12u 28u 95% 18u "Restore missing or damaged program files."
    Pop $0
    ${NSD_CreateRadioButton} 0 56u 100% 12u "&Uninstall GearDot"
    Pop $0
    ${If} $Mode == "remove"
        ${NSD_Check} $0
    ${Else}
        ${NSD_Check} $RepairChoice
    ${EndIf}
    ${NSD_CreateLabel} 12u 72u 95% 18u "Remove GearDot and its shortcuts."
    Pop $0
    nsDialogs::Show
FunctionEnd

Function MaintenanceLeave
    ${NSD_GetState} $RepairChoice $0
    StrCpy $Mode "repair"
    ${If} $0 != ${BST_CHECKED}
        MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "Uninstall GearDot from this computer?" IDYES +2
        Abort
        StrCpy $Mode "remove"
    ${EndIf}
FunctionEnd

Section
    InitPluginsDir
    SetOutPath "$PLUGINSDIR"
    File "GearDot-Setup-x64.msi"
    StrCpy $LogFile "$TEMP\GearDot-Setup.log"

    ${If} $Mode == "remove"
        StrCpy $1 '/x "${PRODUCT_CODE}"'
        DetailPrint "Uninstalling GearDot..."
    ${ElseIf} $Mode == "repair"
        StrCpy $1 '/i "$PLUGINSDIR\GearDot-Setup-x64.msi" REINSTALL=ALL REINSTALLMODE=amus'
        DetailPrint "Repairing GearDot..."
    ${Else}
        StrCpy $0 $INSTDIR
        StrCpy $2 $0 1 -1
        ${If} $2 == "\"
            StrCpy $0 "$0\"
        ${EndIf}
        StrCpy $1 '/i "$PLUGINSDIR\GearDot-Setup-x64.msi" INSTALLFOLDER="$0"'
        DetailPrint "Installing GearDot..."
    ${EndIf}

    ClearErrors
    ExecWait '"$Installer" $1 /qn /norestart /L*v "$LogFile"' $Result
    ${If} ${Errors}
        StrCpy $Result 1
    ${EndIf}
    ${If} $Result == 3010
        SetErrorLevel 3010
        DetailPrint "Restart Windows to complete the operation."
    ${ElseIf} $Result != 0
        MessageBox MB_ICONSTOP "GearDot Setup could not finish (code $Result). See $LogFile." /SD IDOK
        SetErrorLevel $Result
        Abort
    ${EndIf}
SectionEnd

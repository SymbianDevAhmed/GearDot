; Per-user Windows installer for GearDot.
; The release pipeline supplies the version and generated original icon.
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#ifndef AppIcon
  #error AppIcon must point to the generated .ico file
#endif
#define AppName "GearDot"
#define AppPublisher "Ahmed Gaber"
#define ProjectUrl "https://github.com/SymbianDevAhmed/GearDot"
#define AppExe "GearDot.exe"

[Setup]
AppId={{9B085E29-AB9E-4D53-B3E5-0D127EF1E748}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#ProjectUrl}
AppSupportURL={#ProjectUrl}/issues
AppUpdatesURL={#ProjectUrl}/releases
DefaultDirName={localappdata}\Programs\GearDot
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\target\installer
OutputBaseFilename=GearDot-Setup-x64
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} Setup
VersionInfoCopyright=Copyright (c) 2026 Ahmed Gaber
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
AppMutex=Local\WirelessBatteryMonitor.Tray,Local\WirelessBatteryMonitor.Details

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\wireless-battery-monitor.exe"; DestName: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\diagnostic.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\GearDot-DeviceProbe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\HeadsetControl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Settings"; Filename: "{app}\{#AppExe}"; Parameters: "--settings"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch GearDot"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Existing: String;
  InstalledCommand: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    InstalledCommand := '"' + ExpandConstant('{app}\GearDot.exe') + '"';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'WirelessBatteryMonitor', Existing) then
    begin
      if (CompareText(Existing, InstalledCommand) = 0) or
         (CompareText(Existing, InstalledCommand + ' --background') = 0) then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'WirelessBatteryMonitor');
    end;
  end;
end;

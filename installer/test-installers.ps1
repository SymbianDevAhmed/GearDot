param(
    [Parameter(Mandatory)][string]$PackageDirectory
)
$ErrorActionPreference = 'Stop'
$packages = (Resolve-Path -LiteralPath $PackageDirectory).Path
$installed = Join-Path $env:LOCALAPPDATA 'Programs/GearDot'
$shortcut = Join-Path $env:APPDATA 'Microsoft/Windows/Start Menu/Programs/GearDot/GearDot.lnk'
$msiexec = Join-Path $env:SystemRoot 'System32/msiexec.exe'
$msi = Join-Path $packages 'GearDot-Setup-x64.msi'
$exe = Join-Path $packages 'GearDot-Setup-x64.exe'

function Invoke-Installer([string]$Program, [string]$Arguments, [string]$Log) {
    $process = Start-Process -FilePath $Program -ArgumentList $Arguments -PassThru
    if (-not $process.WaitForExit(90000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Installer timed out: $Program"
    }
    if ($process.ExitCode -ne 0) {
        if ($Log -and (Test-Path -LiteralPath $Log)) {
            Select-String -LiteralPath $Log -Pattern 'Return value 3', 'Error [0-9]+', 'MainEngineThread' -Context 8,2 |
                Select-Object -Last 8 | Out-Host
        }
        throw "Installer exit code $($process.ExitCode): $Program"
    }
}

function Test-Install {
    foreach ($name in 'GearDot.exe', 'diagnostic.exe', 'GearDot-DeviceProbe.exe', 'HeadsetControl.exe') {
        if (-not (Test-Path -LiteralPath (Join-Path $installed $name))) {
            throw "Installed file missing: $name"
        }
    }
    if (-not (Test-Path -LiteralPath $shortcut)) { throw 'Start menu shortcut missing.' }
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    if ($target -ne (Join-Path $installed 'GearDot.exe')) { throw "Incorrect shortcut target: $target" }
    $monitor = Start-Process (Join-Path $installed 'GearDot.exe') -ArgumentList '--background' -PassThru
    try {
        if ($monitor.WaitForExit(5000)) { throw "Installed GearDot exited with code $($monitor.ExitCode)." }
    } finally {
        if (!$monitor.HasExited) { Stop-Process -Id $monitor.Id }
        $monitor.Dispose()
    }
}

function Remove-InstalledMsi {
    $log = Join-Path $packages 'uninstall-msi.log'
    Invoke-Installer $msiexec "/x `"$msi`" /qn /norestart /L*v `"$log`"" $log
    if (Test-Path -LiteralPath (Join-Path $installed 'GearDot.exe')) { throw 'Uninstall left GearDot.exe in place.' }
    if (Test-Path -LiteralPath $shortcut) { throw 'Uninstall left the shortcut in place.' }
}

if (Test-Path -LiteralPath (Join-Path $installed 'GearDot.exe')) {
    throw 'Run this test in a disposable Windows environment with no existing GearDot installation.'
}
$log = Join-Path $packages 'install-msi.log'
Invoke-Installer $msiexec "/i `"$msi`" /qn /norestart /L*v `"$log`"" $log
Test-Install
Remove-InstalledMsi
Write-Host 'MSI installed, started GearDot, and uninstalled successfully.'

$log = Join-Path $packages 'install-exe.log'
Invoke-Installer $exe "/S /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=`"$log`"" $log
Test-Install
$innoUninstaller = Get-ChildItem -LiteralPath $installed -Filter 'unins*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($innoUninstaller) {
    Invoke-Installer $innoUninstaller.FullName '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' ''
} else {
    Remove-InstalledMsi
}
Write-Host 'EXE installed, started GearDot, and uninstalled successfully.'

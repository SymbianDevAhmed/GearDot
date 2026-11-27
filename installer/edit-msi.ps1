param(
    [Parameter(Mandatory)][string]$Package,
    [string]$Directory,
    [switch]$ExportSequence,
    [switch]$ProductCode
)
$ErrorActionPreference = 'Stop'
$installer = New-Object -ComObject WindowsInstaller.Installer
$mode = if ($ExportSequence -or $ProductCode) { 0 } else { 1 }
$database = $installer.OpenDatabase($Package, $mode)

if ($ProductCode) {
    $view = $database.OpenView("SELECT Value FROM Property WHERE Property = 'ProductCode'")
    $view.Execute()
    $record = $view.Fetch()
    $record.StringData(1)
    $view.Close()
} elseif ($ExportSequence) {
    $database.Export('InstallUISequence', $Directory, 'InstallUISequence.idt')
} else {
    foreach ($table in Get-ChildItem -LiteralPath $Directory -Filter '*.idt' -File) {
        $database.Import($Directory, $table.Name)
    }
    $database.Commit()
}

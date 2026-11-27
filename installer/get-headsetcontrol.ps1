param([string]$BinDirectory = 'target/release', [string]$SourceDirectory = 'target/release-package')
$ErrorActionPreference = 'Stop'

$version = '4.3.0'
$origin = "https://github.com/Sapd/HeadsetControl/releases/download/$version"
$assets = @(
    @{
        Remote = 'headsetcontrol-windows-x86_64.exe'
        Local = (Join-Path $BinDirectory 'HeadsetControl.exe')
        Sha256 = 'e371da043710ddb8fc70af77f2ccad1cd03f8c6f49ba423e08764ad7a94a3c40'
    },
    @{
        Remote = "headsetcontrol-$version.tar.gz"
        Local = (Join-Path $SourceDirectory "headsetcontrol-$version.tar.gz")
        Sha256 = '918dcacf8aef347744b36b2d259d76880fa64b3869f0f8ef26b6576b873d28a7'
    }
)
foreach ($asset in $assets) {
    New-Item -ItemType Directory -Path (Split-Path $asset.Local -Parent) -Force | Out-Null
    $download = $asset.Local + '.download'
    try {
        Invoke-WebRequest "$origin/$($asset.Remote)" -OutFile $download
        if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $asset.Sha256) {
            throw "HeadsetControl checksum mismatch for $($asset.Remote)"
        }
        Move-Item -LiteralPath $download -Destination $asset.Local -Force
    } finally {
        if (Test-Path -LiteralPath $download) {
            Remove-Item -LiteralPath $download -Force
        }
    }
}

# Builds BatteryHub and writes the release files (the only files users download) to release\:
#   release\BatteryHub-Taskbar-Setup-v1.0.exe
#   release\SHA256SUMS.txt
# cargo prints progress on stderr, so failures are detected through exit codes.
Set-Location $PSScriptRoot

# MSVC link.exe fails (LNK1104) when the build path gets close to MAX_PATH; build elsewhere then.
if (-not $env:CARGO_TARGET_DIR -and $PSScriptRoot.Length -gt 100) {
    $env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'batteryhub-target'
}

cargo build --release --bins
if ($LASTEXITCODE) { throw 'cargo build failed' }

$target = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$release = Join-Path $target 'release'

$env:BH_PAYLOAD_DIR = $release
cargo build --release --manifest-path installer\Cargo.toml
if ($LASTEXITCODE) { throw 'setup build failed' }

# The setup is its own cargo project, so it may build into a different target directory.
$setupTarget = (cargo metadata --format-version 1 --no-deps --manifest-path installer\Cargo.toml | ConvertFrom-Json).target_directory
$setupRelease = Join-Path $setupTarget 'release'

$version = (Get-Content VERSION -Raw).Trim()
$label = 'v' + ($version -replace '\.0$', '')
Remove-Item release -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory release | Out-Null
foreach ($e in 'Taskbar') {
    Copy-Item (Join-Path $setupRelease "BatteryHub-$e-Setup.exe") "release\BatteryHub-$e-Setup-$label.exe"
}
Get-ChildItem release\*.exe | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name } |
    Set-Content release\SHA256SUMS.txt -Encoding ascii

# With a release-signing key (scripts\new-signing-key.ps1), SHA256SUMS.txt is signed: programs
# built with release-signing.pub refuse updates whose checksums are not signed with this key.
if (Test-Path release-signing.pub) {
    Add-Type -AssemblyName System.Core
    $key = [System.Security.Cryptography.CngKey]::Open('BatteryHub Release Signing')
    $ecdsa = New-Object System.Security.Cryptography.ECDsaCng $key
    $ecdsa.HashAlgorithm = [System.Security.Cryptography.CngAlgorithm]::Sha256
    $sums = [IO.File]::ReadAllBytes((Resolve-Path release\SHA256SUMS.txt))
    $sig = $ecdsa.SignData($sums)
    if (-not $ecdsa.VerifyData($sums, $sig)) { throw 'signature check failed' }
    $hex = -join ($sig | ForEach-Object { $_.ToString('x2') })
    [IO.File]::WriteAllText((Join-Path $PWD 'release\SHA256SUMS.txt.sig'), $hex + "`n")
    'Signed SHA256SUMS.txt -> SHA256SUMS.txt.sig'
} else {
    'Not signed (no release-signing.pub; see scripts\new-signing-key.ps1)'
}
Get-ChildItem release | ForEach-Object { '{0,-40} {1,8:N0} KB' -f $_.Name, ($_.Length / 1KB) }

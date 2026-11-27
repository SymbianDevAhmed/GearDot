# Creates the release-signing key (once, on the PC that builds releases).
#
# The private key is an ECDSA P-256 key in this Windows user's key store ("BatteryHub Release
# Signing", Microsoft Software Key Storage Provider), marked non-exportable: it never exists as a
# file and cannot be copied out. Its public half is written to release-signing.pub (hex X || Y),
# which build.rs embeds in the program. From then on:
#   * build-release.ps1 signs SHA256SUMS.txt (SHA256SUMS.txt.sig) with the key;
#   * programs built with release-signing.pub only install updates whose checksums carry a valid
#     signature - an update uploaded by someone else (e.g. after a GitHub account takeover) is
#     refused even if its checksums match.
#
# Commit release-signing.pub. Keep a backup plan in mind: if this Windows profile is lost, the key
# is gone, and users need a manual install of a build with a new key.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)
$name = 'BatteryHub Release Signing'
Add-Type -AssemblyName System.Core
if ([System.Security.Cryptography.CngKey]::Exists($name)) {
    throw "The key '$name' already exists. Delete it first only if you really mean to replace it."
}
$p = New-Object System.Security.Cryptography.CngKeyCreationParameters
$p.ExportPolicy = [System.Security.Cryptography.CngExportPolicies]::None
$p.KeyUsage = [System.Security.Cryptography.CngKeyUsages]::Signing
$key = [System.Security.Cryptography.CngKey]::Create([System.Security.Cryptography.CngAlgorithm]::ECDsaP256, $name, $p)
# BCRYPT_ECCKEY_BLOB: magic (4) | key length (4) | X (32) | Y (32)
$blob = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
$hex = -join ($blob[8..71] | ForEach-Object { $_.ToString('x2') })
[IO.File]::WriteAllText((Join-Path $PWD 'release-signing.pub'), $hex + "`n")
"Created '$name'. Public key written to release-signing.pub:"
$hex

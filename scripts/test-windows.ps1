$ErrorActionPreference = 'Stop'

# Tauri embeds the desktop app manifest in its main binary. Cargo's separate
# unit-test executables also link UI libraries that need Common Controls v6.
$messages = & cargo test --locked --manifest-path src-tauri/Cargo.toml --no-run --message-format=json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$executables = @($messages | ForEach-Object {
    $artifact = $_ | ConvertFrom-Json
    if ($artifact.reason -eq 'compiler-artifact' -and $artifact.profile.test -and $artifact.executable) {
        $artifact.executable
    }
} | Sort-Object -Unique)
if ($executables.Count -eq 0) { throw 'Cargo produced no test executables.' }

$kitRoot = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots').KitsRoot10
$manifestTool = Get-ChildItem (Join-Path $kitRoot 'bin\*\x64\mt.exe') |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
    Select-Object -First 1
if (-not $manifestTool) { throw 'Windows SDK manifest tool (mt.exe) was not found.' }
$manifest = Join-Path $PSScriptRoot 'windows-test.manifest'
foreach ($executable in $executables) {
    Write-Host "Embedding Common Controls v6 manifest in $executable"
    & $manifestTool.FullName -nologo -manifest $manifest "-outputresource:$executable;#1"
    if ($LASTEXITCODE -ne 0) { throw "Manifest embedding failed for $executable" }
}

# Reuse Cargo's normal test selection, harness, and doctest handling. The
# build inputs are unchanged, so Cargo reuses the executable files above.
& cargo test --locked --manifest-path src-tauri/Cargo.toml
exit $LASTEXITCODE

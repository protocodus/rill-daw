$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectDir = Split-Path -Parent $PSScriptRoot
Push-Location $projectDir
$stage = $null
$previousConfig = $env:RILL_CONFIG_DIR
try {
    $hostLine = (& rustc -vV | Select-String '^host:').Line
    if ($LASTEXITCODE -ne 0) { throw 'rustc version query failed' }
    $target = ($hostLine -split '\s+')[1]
    if ($target -ne 'x86_64-pc-windows-msvc') { throw "Unsupported Windows package target: $target" }
    if ($env:RILL_TARGET -and $env:RILL_TARGET -ne $target) { throw "Expected target $env:RILL_TARGET, found $target" }
    if ($env:CARGO_BUILD_TARGET -and $env:CARGO_BUILD_TARGET -ne $target) { throw 'Native packaging cannot cross compile' }
    if (-not $env:RUSTFLAGS) { $env:RUSTFLAGS = '-C target-feature=+crt-static' }
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $metadata = $metadataText | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'daw-app').version
    & cargo build --locked --release --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    $binDir = Join-Path $metadata.target_directory 'release'
    if ($env:CARGO_BUILD_TARGET) { $binDir = Join-Path $metadata.target_directory "$target/release" }
    $name = "rill-$version-$target"
    $stage = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
    $packageDir = Join-Path $stage $name
    New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
    Copy-Item (Join-Path $binDir 'rill.exe'), (Join-Path $binDir 'daw-plugin-host.exe'), 'README.md', 'LICENSE' -Destination $packageDir
    Copy-Item 'docs' -Destination $packageDir -Recurse
    & python scripts/verify-branding.py --windows (Join-Path $packageDir 'rill.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Embedded Windows icon verification failed' }
    & python scripts/collect-licenses.py $packageDir
    if ($LASTEXITCODE -ne 0) { throw 'License inventory failed' }
    @'
Rill for Windows x64

Extract the complete directory and launch rill.exe. Keep daw-plugin-host.exe
beside the application. The CI build statically links the host's MSVC runtime;
separately installed VST3 plug-ins may require their own vendor runtimes.
Windows 10/11 x64 and an OpenGL-capable graphics driver are required.
This build is not Authenticode signed, so Windows may ask for first-run approval.
No installer, registry changes, or administrator privileges are needed.
'@ | Set-Content (Join-Path $packageDir 'INSTALL.txt') -Encoding utf8
    $env:RILL_CONFIG_DIR = Join-Path $stage 'smoke-config'
    & (Join-Path $packageDir 'rill.exe') --render-demo (Join-Path $stage 'smoke.wav')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path (Join-Path $stage 'smoke.wav'))) { throw 'Packaged executable render smoke test failed' }
    $dist = Join-Path $projectDir 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    $archive = Join-Path $dist "$name.zip"
    Compress-Archive -Path $packageDir -DestinationPath $archive -Force
    $digest = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$archive.sha256", "$digest  $name.zip`n", [Text.UTF8Encoding]::new($false))
    if ($env:GITHUB_ENV) {
        $verifyDir = Join-Path $dist "verify-$target-$([Guid]::NewGuid().ToString('N'))"
        Expand-Archive -Path $archive -DestinationPath $verifyDir -Force
        $verifiedExecutable = Join-Path $verifyDir "$name/rill.exe"
        if (-not (Test-Path $verifiedExecutable)) { throw 'Extracted archive is missing rill.exe' }
        if (-not (Test-Path (Join-Path $verifyDir "$name/daw-plugin-host.exe"))) { throw 'Extracted archive is missing daw-plugin-host.exe' }
        [IO.File]::AppendAllText($env:GITHUB_ENV, "RILL_E2E_BINARY=$verifiedExecutable`n", [Text.UTF8Encoding]::new($false))
    }
    Write-Output "Packaged $archive"
} finally {
    $env:RILL_CONFIG_DIR = $previousConfig
    if ($stage -and (Test-Path $stage)) { Remove-Item $stage -Recurse -Force }
    Pop-Location
}

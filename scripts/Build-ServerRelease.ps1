param(
    [string]$TargetDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target'),
    [string]$OutputDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target'),
    [switch]$WithManaged
)
$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path $PSScriptRoot -Parent
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Build-ServerRelease.ps1 requires Windows with the MSVC x64 Rust toolchain.'
}

Push-Location $ProjectRoot
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$stage = $null
try {
    # PowerShell's location can differ from the process working directory used
    # by GetFullPath. Resolve relative overrides against the repository first.
    if (-not [IO.Path]::IsPathRooted($TargetDirectory)) {
        $TargetDirectory = Join-Path $ProjectRoot $TargetDirectory
    }
    if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
        $OutputDirectory = Join-Path $ProjectRoot $OutputDirectory
    }
    $TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
    $OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
    $revision = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify the source revision' }
    $sourceStatus = @(& git status --porcelain --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify local source modifications' }
    # Encoded flags take precedence over ambient RUSTFLAGS and target config.
    # Link the CRT statically so the extracted server needs no redistributable.
    $env:CARGO_ENCODED_RUSTFLAGS = '-C' + [char]0x1f + 'target-feature=+crt-static'
    $stage = Join-Path $TargetDirectory ('server-release-stage/' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    $executable = Join-Path $stage 'skate-server.exe'
    # Emit this invocation into a unique stage, never a stale cached executable.
    & cargo rustc --release --locked --target x86_64-pc-windows-msvc `
        --target-dir $TargetDirectory -p skate-server --bin skate-server -- `
        -C extra-filename= -C debuginfo=0 --emit "link=$executable"
    if ($LASTEXITCODE -ne 0) { throw 'Dedicated server compilation failed' }
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw 'Fresh dedicated server executable is missing'
    }
    & $executable --help
    if ($LASTEXITCODE -ne 0) { throw 'Dedicated server executable smoke check failed' }
    $accountExecutable = Join-Path $stage 'skate-account.exe'
    & cargo rustc --release --locked --target x86_64-pc-windows-msvc `
        --target-dir $TargetDirectory -p skate-accounts --bin skate-account -- `
        -C extra-filename= -C debuginfo=0 --emit "link=$accountExecutable"
    if ($LASTEXITCODE -ne 0) { throw 'Account setup tool compilation failed' }
    $packageArguments = @(
        (Join-Path $ProjectRoot 'tools/package_server.py'),
        '--executable', $executable, '--output-directory', $OutputDirectory,
        '--revision', $revision.Trim(), '--account-executable', $accountExecutable
    )
    if ($WithManaged) {
        $managed = Join-Path $stage 'managed-host'
        & dotnet publish (Join-Path $ProjectRoot 'crates/skate-mods/managed-host/Skate.ResourceHost.csproj') `
            -c Release -o $managed --nologo -p:UseAppHost=false
        if ($LASTEXITCODE -ne 0) { throw 'Managed resource host publication failed' }
        $dotnetRoot = Split-Path (Get-Command dotnet -ErrorAction Stop).Source -Parent
        foreach ($notice in @('LICENSE.txt', 'ThirdPartyNotices.txt')) {
            Copy-Item -LiteralPath (Join-Path $dotnetRoot $notice) -Destination $managed
        }
        $packageArguments += @('--managed-host', $managed)
    }
    if ($sourceStatus.Count -gt 0) { $packageArguments += '--source-modified' }
    & python @packageArguments
    if ($LASTEXITCODE -ne 0) { throw 'Dedicated server packaging failed' }
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    if ($stage -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    Pop-Location
}

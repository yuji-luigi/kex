# Install the latest kex release: irm https://raw.githubusercontent.com/yuji-luigi/kex/main/install.ps1 | iex
# $env:KEX_INSTALL_DIR overrides the folder (default %LOCALAPPDATA%\Programs\kex).
$ErrorActionPreference = 'Stop'
$repo = 'yuji-luigi/kex'
$dir = if ($env:KEX_INSTALL_DIR) { $env:KEX_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\kex' }
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
$name = "kex-$arch-pc-windows-msvc.zip"
$url = "https://github.com/$repo/releases/latest/download/$name"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "downloading $name"
    Invoke-WebRequest $url -OutFile "$tmp\$name"
    Invoke-WebRequest "$url.sha256" -OutFile "$tmp\$name.sha256"
    $expected = ((Get-Content "$tmp\$name.sha256") -split '\s+')[0].ToLower()
    $actual = (Get-FileHash "$tmp\$name" -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw 'kex: checksum mismatch, not installing' }
    Expand-Archive "$tmp\$name" -DestinationPath $tmp
    New-Item -ItemType Directory -Force $dir | Out-Null
    Move-Item -Force "$tmp\kex.exe" "$dir\kex.exe"
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
        Write-Host "added $dir to your PATH (open a new terminal)"
    }
    Write-Host "installed $(& "$dir\kex.exe" --version) to $dir\kex.exe"
} finally {
    Remove-Item -Recurse -Force $tmp
}

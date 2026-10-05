# One-click install for Windows:
#   irm https://raw.githubusercontent.com/GitDjAr/t-translate/main/install.ps1 | iex
# Optional env: T_MIRROR (download prefix, e.g. https://ghfast.top/), T_INSTALL_DIR
$ErrorActionPreference = 'Stop'
$repo = 'GitDjAr/t-translate'
$asset = 't-windows-x64.exe'
$dir = if ($env:T_INSTALL_DIR) { $env:T_INSTALL_DIR } else { Join-Path $HOME '.t-translate\bin' }
$url = "$($env:T_MIRROR)https://github.com/$repo/releases/latest/download/$asset"

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$exe = Join-Path $dir 't.exe'
$tmp = "$exe.new"

Write-Host "Downloading $url"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing

if (Test-Path $exe) {
    # a running exe can be renamed but not overwritten
    Remove-Item "$exe.old" -Force -ErrorAction SilentlyContinue
    Move-Item $exe "$exe.old" -Force
}
Move-Item $tmp $exe -Force

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$dir"), 'User')
    Write-Host "Added $dir to your user PATH (reopen the terminal)."
}
$env:Path += ";$dir"

& $exe --version
Write-Host "Done. Try:  t git -h     (rename the command with: t --alias)"

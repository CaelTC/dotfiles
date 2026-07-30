# claude-analyzer — download and run.
#   irm https://raw.githubusercontent.com/CaelTC/dotfiles/main/claude-analyzer/install.ps1 | iex
#
# Reads the Claude Code transcripts already on this machine
# (%USERPROFILE%\.claude\projects), writes an HTML report to your temp folder,
# and opens it in your browser. Nothing is uploaded anywhere.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # PS 5.1 draws a progress bar per byte otherwise
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$url = 'https://github.com/CaelTC/dotfiles/releases/latest/download/claude-analyzer.exe'
$exe = Join-Path $env:TEMP 'claude-analyzer.exe'

Write-Host "Downloading claude-analyzer..."
Invoke-WebRequest -Uri $url -OutFile $exe

Write-Host "Reading your transcripts..."
& $exe

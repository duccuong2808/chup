# Tải một gói OCR trong crates/core/src/ocr/manifest.json (data | engine) vào thư mục chỉ định, kiểm kích thước
# và SHA-256 từng file. CI dùng để đóng gói bộ cài Full; chạy tay được trên Windows.
#   pwsh installer/fetch-ocr.ps1 -Pack engine -Out ocr-dist
#   pwsh installer/fetch-ocr.ps1 -Pack data -Out ocr-data
# In ra tên thư mục của gói (vd. vi-1) ở dòng cuối.
param(
  [Parameter(Mandatory)][ValidateSet('data', 'engine')][string]$Pack,
  [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
$manifest = Get-Content -Raw -Encoding utf8 (Join-Path $PSScriptRoot '..\crates\core\src\ocr\manifest.json') | ConvertFrom-Json
$p = $manifest.$Pack
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($f in $p.files) {
  $dst = Join-Path $Out $f.name
  for ($try = 1; ; $try++) {
    try { Invoke-WebRequest -UseBasicParsing "$($p.baseUrl)$($f.name)" -OutFile $dst; break }
    catch { if ($try -ge 3) { throw }; Start-Sleep 5 }
  }
  $size = (Get-Item $dst).Length
  $sha = (Get-FileHash -Algorithm SHA256 $dst).Hash.ToLower()
  if ($size -ne $f.size -or $sha -ne $f.sha256) {
    throw "$($f.name): $size byte, sha256 $sha; manifest ghi $($f.size) byte, $($f.sha256)"
  }
  Write-Host "$($f.name): $size byte, sha256 khớp manifest"
}
$p.dir

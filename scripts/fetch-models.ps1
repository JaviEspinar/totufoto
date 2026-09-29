# Downloads the InsightFace "buffalo_s" face models into .\models (Windows).
# Note: InsightFace pretrained models are licensed for non-commercial research use.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
if ((Test-Path "models\det_500m.onnx") -and (Test-Path "models\w600k_mbf.onnx")) { Write-Host "models already present"; exit 0 }
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Invoke-WebRequest "https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_s.zip" -OutFile "$tmp\buffalo_s.zip"
    Expand-Archive "$tmp\buffalo_s.zip" -DestinationPath $tmp
    New-Item -ItemType Directory -Force models | Out-Null
    $src = Get-ChildItem $tmp -Recurse -Include det_500m.onnx, w600k_mbf.onnx
    $src | Copy-Item -Destination models\
    Write-Host "models ready in .\models"
} finally {
    Remove-Item -Recurse -Force $tmp
}

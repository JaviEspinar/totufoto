# Downloads Microsoft's official ONNX Runtime CPU build into .\onnxruntime (Windows x64).
# The official build picks SSE/AVX/AVX2/AVX-512 kernels at runtime, so it also runs on CPUs without AVX2.
$ErrorActionPreference = "Stop"
$version = if ($env:ORT_VERSION) { $env:ORT_VERSION } else { "1.28.2" }
Set-Location (Join-Path $PSScriptRoot "..")

if (Test-Path "onnxruntime\onnxruntime.dll") { Write-Host "ONNX Runtime already present in .\onnxruntime"; exit 0 }
$pkg = "onnxruntime-win-x64-$version"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Invoke-WebRequest "https://github.com/microsoft/onnxruntime/releases/download/v$version/$pkg.zip" -OutFile "$tmp\ort.zip"
    Expand-Archive "$tmp\ort.zip" -DestinationPath $tmp
    New-Item -ItemType Directory -Force onnxruntime | Out-Null
    Copy-Item "$tmp\$pkg\lib\onnxruntime.dll" onnxruntime\
    Copy-Item "$tmp\$pkg\LICENSE" onnxruntime\
    Write-Host "ONNX Runtime $version ready in .\onnxruntime"
} finally {
    Remove-Item -Recurse -Force $tmp
}

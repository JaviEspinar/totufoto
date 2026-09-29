# Downloads Microsoft's official ONNX Runtime CPU build into .\onnxruntime (Windows x64).
# The official build picks SSE/AVX/AVX2/AVX-512 kernels at runtime, so it also runs on CPUs without AVX2.
$ErrorActionPreference = "Stop"
$version = if ($env:ORT_VERSION) { $env:ORT_VERSION } else { "1.28.2" }
Set-Location (Join-Path $PSScriptRoot "..")

# onnxruntime.dll needs the Visual C++ runtime. Copies next to it are loaded first, so the
# app also runs where the VC++ redistributable isn't installed (app-local deployment).
function Copy-VcRuntime {
    $names = "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll"
    $source = $null
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vs = & $vswhere -latest -products * -property installationPath
        if ($vs) {
            $source = Get-ChildItem "$vs\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue |
                Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
        }
    }
    if (-not $source) { $source = "$env:WINDIR\System32" }
    foreach ($n in $names) {
        if (-not (Test-Path "$source\$n")) { throw "$n not found in $source; install the Visual C++ redistributable or Visual Studio Build Tools" }
        Copy-Item "$source\$n" onnxruntime\
    }
    Write-Host "Visual C++ runtime copied from $source"
}

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
    Copy-VcRuntime
    Write-Host "ONNX Runtime $version ready in .\onnxruntime"
} finally {
    Remove-Item -Recurse -Force $tmp
}

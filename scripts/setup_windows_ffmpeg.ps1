param([string]$Destination = "$PSScriptRoot/../target/vendor/ffmpeg")
$ErrorActionPreference = 'Stop'
$spec = Get-Content "$PSScriptRoot/windows-ffmpeg.json" | ConvertFrom-Json
if (Test-Path $Destination) { throw "Destination already exists: $Destination" }
$parent = Split-Path ([IO.Path]::GetFullPath($Destination))
New-Item -ItemType Directory -Force $parent | Out-Null
$archive = Join-Path $parent 'ffmpeg-windows.zip'
Invoke-WebRequest -Uri $spec.url -OutFile $archive
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $spec.sha256) {
    throw 'FFmpeg archive SHA-256 mismatch'
}
$expanded = Join-Path $parent 'ffmpeg-expanded'
Expand-Archive $archive $expanded
$root = @(Get-ChildItem $expanded -Directory)
if ($root.Count -ne 1) { throw 'Unexpected FFmpeg archive layout' }
Move-Item $root[0].FullName $Destination
Remove-Item $expanded
$Destination = [IO.Path]::GetFullPath($Destination)
# Some MinGW distributions provide .dll.a only. Generate MSVC import libraries
# from the actual DLL exports, rather than linking a different FFmpeg build.
foreach ($library in 'avcodec','avformat','avutil','swresample','swscale') {
    $import = Join-Path $Destination "lib/$library.lib"
    if (-not (Test-Path $import)) {
        $dll = @(Get-ChildItem "$Destination/bin/$library-*.dll")
        if ($dll.Count -ne 1) { throw "Missing or ambiguous $library DLL" }
        $exports = & dumpbin /nologo /exports $dll[0].FullName
        if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for $library" }
        $names = @($exports | ForEach-Object {
            if ($_ -match '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\S+)') { $Matches[1] }
        })
        if ($names.Count -eq 0) { throw "No exports in $library" }
        $definition = Join-Path $Destination "lib/$library.def"
        @("LIBRARY $($dll[0].Name)", 'EXPORTS') + $names | Set-Content -Encoding ascii $definition
        & lib /nologo /machine:x64 "/def:$definition" "/out:$import"
        if ($LASTEXITCODE -ne 0) { throw "lib.exe failed for $library" }
    }
}
$env:FFMPEG_DIR = $Destination
$env:PATH = "$Destination/bin;$env:PATH"
if ($env:GITHUB_ENV) { "FFMPEG_DIR=$Destination" | Out-File -Append -Encoding utf8 $env:GITHUB_ENV }
if ($env:GITHUB_PATH) { "$Destination/bin" | Out-File -Append -Encoding utf8 $env:GITHUB_PATH }
Write-Host "Verified FFmpeg: $Destination"

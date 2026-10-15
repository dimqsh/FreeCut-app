param([string]$Package = '', [int]$LimitMiB = 1536)
$ErrorActionPreference = 'Stop'
if (-not $Package) {
    $Package = (Get-ChildItem "$PSScriptRoot/../target/windows-dist" -Directory -Filter 'FreeCut-*')[0].FullName
}
$output = [IO.Path]::GetFullPath("$PSScriptRoot/../target/windows-check")
New-Item -ItemType Directory -Force $output | Out-Null
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class FreeCutJob {
    [StructLayout(LayoutKind.Sequential)] public struct Basic {
        public long ProcessTime, JobTime;
        public uint Flags;
        public UIntPtr MinWorkingSet, MaxWorkingSet;
        public uint ActiveProcesses;
        public UIntPtr Affinity;
        public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Counters {
        public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Extended {
        public Basic Basic;
        public Counters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr CreateJobObject(IntPtr a, string n);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool SetInformationJobObject(IntPtr j, int c, ref Extended i, uint s);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool QueryInformationJobObject(IntPtr j, int c, out Extended i, uint s, IntPtr r);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool AssignProcessToJobObject(IntPtr j, IntPtr p);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
}
'@
$job = [FreeCutJob]::CreateJobObject([IntPtr]::Zero, $null)
if ($job -eq [IntPtr]::Zero) { throw 'CreateJobObject failed' }
$limits = New-Object FreeCutJob+Extended
$limits.Basic.Flags = 0x2100 # PROCESS_MEMORY + KILL_ON_JOB_CLOSE
$limits.ProcessMemory = [UIntPtr]([uint64]$LimitMiB * 1MB)
$size = [Runtime.InteropServices.Marshal]::SizeOf($limits)
if (-not [FreeCutJob]::SetInformationJobObject($job, 9, [ref]$limits, $size)) {
    throw "SetInformationJobObject failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
}
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = Join-Path $Package 'freecut.exe'
$start.Arguments = '--software --low-memory'
$start.WorkingDirectory = $Package
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Environment['APPDATA'] = Join-Path $output 'profile'
$start.Environment['FREECUT_LOW_MEMORY'] = '1'
$start.Environment['WGPU_BACKEND'] = 'dx12'
$process = New-Object Diagnostics.Process
$process.StartInfo = $start
$result = $null
try {
    if (-not $process.Start()) { throw 'Failed to start FreeCut' }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not [FreeCutJob]::AssignProcessToJobObject($job, $process.Handle)) {
        throw "AssignProcessToJobObject failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
    $process.ProcessorAffinity = [IntPtr]3
    if (-not $process.WaitForInputIdle(30000)) { throw 'FreeCut did not enter its GUI event loop' }
    Start-Sleep -Seconds 15
    $process.Refresh()
    if ($process.HasExited) { throw "FreeCut exited early: $($process.ExitCode)" }
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'No FreeCut window was created' }
    $measured = New-Object FreeCutJob+Extended
    if (-not [FreeCutJob]::QueryInformationJobObject($job, 9, [ref]$measured, $size, [IntPtr]::Zero)) {
        throw 'Cannot read process memory peak'
    }
    $result = [ordered]@{
        os = [Environment]::OSVersion.VersionString
        memory_limit_bytes = [uint64]$limits.ProcessMemory
        memory_limit_type = 'Windows Job Object per-process committed memory; not whole OS RAM'
        peak_committed_bytes = [uint64]$measured.PeakProcessMemory
        working_set_bytes = $process.WorkingSet64
        peak_working_set_bytes = $process.PeakWorkingSet64
        cpu_affinity_mask = 3
        window_title = $process.MainWindowTitle
        window_created = $true
        limitations = @('Hosted Windows Server runner, not old physical hardware', 'Startup/Home only under the Job Object limit; editing/export covered by separate compact tests', 'Windows 7/8 and 32-bit Windows not validated')
    }
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(10000)) { throw 'FreeCut did not close normally' }
    $text = $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()
    $text | Set-Content "$output/cpu-start.log"
    if ($process.ExitCode -ne 0) { throw "FreeCut exited with $($process.ExitCode): $text" }
    if ($text -notmatch 'FreeCut software UI:.*Microsoft Basic Render Driver') {
        throw "WARP selection was not confirmed: $text"
    }
    if ($text -notmatch 'low_memory: true') { throw "Compact budget was not confirmed: $text" }
    $result.adapter = 'Microsoft Basic Render Driver (WARP); selector accepts DeviceType::Cpu only'
    $result.normal_exit = $true
    $result | ConvertTo-Json -Depth 5 | Set-Content "$output/cpu-result.json"
    $result | ConvertTo-Json -Depth 5 | Write-Host
} finally {
    if ($process.Id -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    if ($stderr) {
        ($stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()) | Set-Content "$output/cpu-start.log"
    }
    [FreeCutJob]::CloseHandle($job) | Out-Null
    $process.Dispose()
}

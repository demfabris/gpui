param([string]$Exe, [string]$Mode = "static", [int]$Seconds = 20, [string]$Log = "")
$bin = "D:\dev\gpui-lab-bin"
$name = [IO.Path]::GetFileNameWithoutExtension($Exe)
Get-Process -Name $name -ErrorAction SilentlyContinue | Stop-Process -Force
$cmd = "$bin\run.cmd"
"@echo off`r`nset FRAME_LAB=$Mode`r`nset FRAME_LAB_LOG=$Log`r`nstart `"`" `"$Exe`"`r`n" | Set-Content -Encoding ASCII $cmd
$action = New-ScheduledTaskAction -Execute $cmd
$principal = New-ScheduledTaskPrincipal -UserId "fabri" -LogonType Interactive
Register-ScheduledTask -TaskName gpuilab -Action $action -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName gpuilab
Start-Sleep 6
$p = Get-Process -Name $name -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "no process"; Unregister-ScheduledTask -TaskName gpuilab -Confirm:$false; exit 1 }
$t0 = $p.TotalProcessorTime.TotalMilliseconds
$w0 = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep $Seconds
$p.Refresh()
$t1 = $p.TotalProcessorTime.TotalMilliseconds
$wall = $w0.Elapsed.TotalMilliseconds
"{0} {1} session={2} cpu_pct_of_core={3:N3}" -f $Exe, $Mode, $p.SessionId, (100.0 * ($t1 - $t0) / $wall)
Stop-Process -Id $p.Id -Force
Unregister-ScheduledTask -TaskName gpuilab -Confirm:$false

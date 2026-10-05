#!/usr/bin/env bash
# Runtime evidence for lab/windows that was NOT RUN while win-desktop was off.
# Run from the Mac once `ssh fabri@win-desktop` answers. Never touches D:\dev\zz or D:\dev\zz-ci.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
W=fabri@win-desktop
BASE=499a950
HEAD=$(git -C ~/dev/gpui-lab/windows rev-parse lab/windows)
TC='$env:PATH = "C:\Users\fabri\.rustup\toolchains\1.97.0-x86_64-pc-windows-msvc\bin;$env:PATH"'
win() { ssh "$W" "$TC; $1" 2>&1 | grep -v -i 'post-quantum\|store now, decrypt\|openssh.com/pq'; }

# 1. clone + branch + scripts
ssh "$W" 'if (-not (Test-Path D:\dev\gpui-lab)) { git clone https://github.com/demfabris/gpui D:\dev\gpui-lab }; New-Item -ItemType Directory -Force D:\dev\gpui-lab-bin | Out-Null'
git -C ~/dev/gpui-lab/windows push "$W:D:/dev/gpui-lab" lab/windows:lab/windows
scp "$HERE/build.ps1" "$HERE/measure.ps1" "$HERE/frame_lab.rs" "$W:D:/dev/gpui-lab-bin/"
win 'quser; Get-WinUserLanguageList | Select-Object LanguageTag, InputMethodTips'

# 2. gates at the branch head (each under 10 min; run detached with Win32_Process if they are not)
win "Set-Location D:\dev\gpui-lab; git checkout --quiet lab/windows; cargo test -j6 -p gpui_windows --lib 2>&1 | Select-Object -Last 30; \"exit \$LASTEXITCODE\""
win "Set-Location D:\dev\gpui-lab; cargo check -j6 --workspace 2>&1 | Select-Object -Last 5; \"exit \$LASTEXITCODE\""

# 3. IME decision test fails with the old query (restore the file afterwards)
win "Set-Location D:\dev\gpui-lab; (Get-Content crates\gpui_windows\src\events.rs -Raw).Replace('input_handler.query_prefers_ime_for_printable_keys()', 'input_handler.query_accepts_text_input()') | Set-Content -NoNewline crates\gpui_windows\src\events.rs; cargo test -j6 -p gpui_windows --lib ime_stays_off 2>&1 | Select-Object -Last 12; git checkout -- crates\gpui_windows\src\events.rs; git status --short"

# 4. release builds of hello_world + frame_lab at base and head (detached; poll build-*.done)
for pair in "$BASE base" "$HEAD head"; do set -- $pair
  win "Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine='powershell -NoProfile -ExecutionPolicy Bypass -File D:\dev\gpui-lab-bin\build.ps1 -Rev $1 -Out $2'} | Out-Null"
  until ssh "$W" "Test-Path D:\dev\gpui-lab-bin\build-$2.done" 2>/dev/null | grep -q True; do sleep 30; done
  win "Get-Content D:\dev\gpui-lab-bin\build-$2.done"
done

# 5. idle CPU, 5 runs x 20 s each, in the logged-on session through a scheduled task
for build in base head; do for exe in hello_world frame_lab; do for run in 1 2 3 4 5; do
  win "powershell -NoProfile -ExecutionPolicy Bypass -File D:\dev\gpui-lab-bin\measure.ps1 -Exe D:\dev\gpui-lab-bin\\$build\\$exe.exe -Mode static -Seconds 20"
done; done; done

# 6. frame pacing: anim (request_animation_frame), timer16 (16 ms timer + notify), wake (notify every 250 ms -> latency_ms)
for build in base head; do for mode in anim timer16 wake; do
  win "Remove-Item D:\dev\gpui-lab-bin\\$build-$mode.log -ErrorAction SilentlyContinue; powershell -NoProfile -ExecutionPolicy Bypass -File D:\dev\gpui-lab-bin\measure.ps1 -Exe D:\dev\gpui-lab-bin\\$build\frame_lab.exe -Mode $mode -Seconds 20 -Log D:\dev\gpui-lab-bin\\$build-$mode.log"
  scp "$W:D:/dev/gpui-lab-bin/$build-$mode.log" "$HERE/"
done; done
python3 - "$HERE" <<'PY'
import statistics, sys, pathlib
for log in sorted(pathlib.Path(sys.argv[1]).glob('*-*.log')):
    fps = [float(l.split()[1]) for l in log.read_text().splitlines() if l.startswith('fps')][2:]
    lat = [float(l.split()[1]) for l in log.read_text().splitlines() if l.startswith('latency_ms')][4:]
    out = [log.stem]
    if fps: out.append(f"fps median {statistics.median(fps):.1f} min {min(fps):.1f} max {max(fps):.1f}")
    if lat: out.append(f"notify->render ms median {statistics.median(lat):.2f} p90 {sorted(lat)[int(len(lat)*.9)]:.2f} max {max(lat):.2f}")
    print('  '.join(out))
PY
# 7. leftovers: measure.ps1 unregisters its task; check none remain
win 'Get-ScheduledTask -TaskName gpuilab -ErrorAction SilentlyContinue; Get-Process frame_lab, hello_world -ErrorAction SilentlyContinue'

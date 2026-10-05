param([string]$Rev, [string]$Out)
$env:PATH = "C:\Users\fabri\.rustup\toolchains\1.97.0-x86_64-pc-windows-msvc\bin;$env:PATH"
$bin = "D:\dev\gpui-lab-bin"
Set-Location D:\dev\gpui-lab
Remove-Item "$bin\build-$Out.done" -ErrorAction SilentlyContinue
git checkout --quiet --detach $Rev *> "$bin\build-$Out.log"
Copy-Item "$bin\frame_lab.rs" crates\gpui\examples\frame_lab.rs -Force
cargo build -j6 --release -p gpui --example hello_world --example frame_lab *>> "$bin\build-$Out.log"
$code = $LASTEXITCODE
New-Item -ItemType Directory -Force "$bin\$Out" | Out-Null
Copy-Item target\release\examples\hello_world.exe, target\release\examples\frame_lab.exe "$bin\$Out\" -Force
Remove-Item crates\gpui\examples\frame_lab.rs
"exit $code" | Out-File "$bin\build-$Out.done"

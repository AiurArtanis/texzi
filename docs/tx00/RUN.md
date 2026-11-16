# TX-00 private test candidate

This is a native NSH window baseline, not an editor. No document opening,
editing, saving, recovery, Markdown or installer is implemented.

Extract the entire ZIP, then in PowerShell 7 run:

```powershell
Get-FileHash ./texzi-app.exe -Algorithm SHA256
./texzi-app.exe 2> ./startup.log
$LASTEXITCODE
```

Compare the hash with SHA256SUMS.txt. Record the exact Windows build, commit,
CI run, exe hash and package hash before testing. Check window appearance,
move, resize, close and process exit. On startup failure capture stderr and
dismiss any NSH error dialog; record exit code and remaining processes.
Do not infer a runtime result from the build log.

Windows x64 only. The test ZIP does not bundle fonts or redistributable runtime
DLLs; Windows Direct2D/DirectWrite and system fonts are used. MSVC runtime
availability must be checked on the target machine; if a runtime is missing,
record the loader error rather than silently installing anything. This is an
unsigned private test package. Windows 10 22H2 x64 clean build and runtime
remain required for G0; 1809 is a separate unverified target.

Application code performs no user document IO. NSH may maintain framework
state/cache and register native application identity; this baseline does not
claim to be filesystem-write-free. Failure injection and user-document
non-modification checks remain independent runtime verification work.

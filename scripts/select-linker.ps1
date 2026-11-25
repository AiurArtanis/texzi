# Explicit final-link selection, later checked against rustc's emitted invocation.
$msvcRoot = Join-Path $vs ("VC/Tools/MSVC/" + $defaultMsvc.Trim())
$effectiveLinker = (Resolve-Path (Join-Path $msvcRoot 'bin/Hostx64/x64/link.exe')).Path
$sdkRoot = "${env:ProgramFiles(x86)}\Windows Kits\10"
$sdkVersion = Get-ChildItem (Join-Path $sdkRoot 'Lib') -Directory |
    Where-Object { $_.Name -match '^10\.0\.\d+\.0$' -and
        (Test-Path (Join-Path $_.FullName 'um/x64/kernel32.lib')) -and
        (Test-Path (Join-Path $_.FullName 'ucrt/x64/libucrt.lib')) } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1 -ExpandProperty Name
if (!$sdkVersion) { throw 'No complete Windows SDK library selection available' }
$selectedLibs = @((Join-Path $msvcRoot 'lib/x64'),
    (Join-Path $sdkRoot "Lib/$sdkVersion/ucrt/x64"),
    (Join-Path $sdkRoot "Lib/$sdkVersion/um/x64")) | ForEach-Object { (Resolve-Path $_).Path }
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $effectiveLinker
$env:LIB = $selectedLibs -join ';'
$flags = @('-C', 'target-feature=+crt-static', '--print=link-args')
foreach ($libraryPath in $selectedLibs) { $flags += @('-C', "link-arg=/LIBPATH:$libraryPath") }
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
$linkerFile = Get-Item $effectiveLinker
@{ linker=$effectiveLinker; linker_file_version=$linkerFile.VersionInfo.FileVersion;
    linker_sha256=(Get-FileHash $effectiveLinker).Hash; msvc=$defaultMsvc.Trim();
    sdk=$sdkVersion; explicit_libpaths=$selectedLibs; lib_environment=$env:LIB; rustflags=$flags;
    boundary='Requested final-link selection; effective proof is final-app-link-command.txt plus successful cargo build. Does not trace every C compiler/header or OS loader.'
} | ConvertTo-Json -Depth 5 | Set-Content build-evidence/effective-toolchain-config.json
# link.exe /? returns 1100 rather than success on this MSVC version.
# Use the actual selected file's version resource/hash, then verify its
# invocation in the successful build; do not weaken native exit handling.

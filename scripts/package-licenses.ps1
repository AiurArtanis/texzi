function Copy-DependencyLicenses {
    param(
        [Parameter(Mandatory)] [psobject] $Metadata,
        [Parameter(Mandatory)] [string] $Destination,
        [Parameter(Mandatory)] [string] $ProjectLicense
    )

    foreach ($package in $Metadata.packages) {
        $folder = Join-Path $Destination "$($package.name)-$($package.version)"
        New-Item -Path $folder -ItemType Directory -Force | Out-Null
        $root = Split-Path $package.manifest_path -Parent
        if ($package.name -eq 'pcre2-sys') {
            $nativeLicense = Join-Path $PSScriptRoot '../crates/texzi-search/PCRE2-LICENCE.md'
            Copy-Item -LiteralPath $nativeLicense -Destination (Join-Path $folder 'PCRE2-10.46-LICENCE.md')
            Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../crates/texzi-search/SLJIT-LICENSE') -Destination $folder
        }
        if ($package.license_file) {
            Copy-Item -LiteralPath (Join-Path $root $package.license_file) -Destination $folder
        }
        Get-ChildItem -LiteralPath $root -File |
            Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' |
            Copy-Item -Destination $folder
        if ($package.name -eq 'nsh-base') {
            Copy-Item -LiteralPath (Join-Path $root 'fonts/katex.LICENSE.txt') -Destination $folder
        }
        if ($package.name -eq 'mac') {
            Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $folder
            Copy-Item -LiteralPath $ProjectLicense -Destination (Join-Path $folder 'Apache-2.0.txt')
        }
        # The Git dependencies from NSH inherit the checkout's root license.
        if ($package.source -like 'git+https://github.com/longbridge/nsh*') {
            $repoRoot = Split-Path (Split-Path $root -Parent) -Parent
            Get-ChildItem -LiteralPath $repoRoot -File |
                Where-Object Name -Match '^(LICENSE|NOTICE)' |
                Copy-Item -Destination $folder -Force
        }
        $component = [ordered]@{
            name = $package.name; version = $package.version; source = $package.source
            license = $package.license; license_file = $package.license_file; id = $package.id
        }
        if ($package.name -eq 'pcre2-sys') {
            $component['bundled'] = @(
                [ordered]@{name='PCRE2'; version='10.46'; unicode='16.0.0'; license='BSD-3-Clause WITH PCRE2-exception'; license_file='PCRE2-10.46-LICENCE.md'}
                [ordered]@{name='SLJIT'; revision='e51eabbfb8eabc6526f56e4e88b29fb10d1ee048'; license='BSD-2-Clause'; license_file='SLJIT-LICENSE'}
            )
        }
        $component
    }
}

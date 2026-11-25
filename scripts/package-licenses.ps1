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
        if ($package.license_file) {
            Copy-Item -LiteralPath (Join-Path $root $package.license_file) -Destination $folder
        }
        Get-ChildItem -LiteralPath $root -File |
            Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' |
            Copy-Item -Destination $folder
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
        [ordered]@{
            name = $package.name; version = $package.version; source = $package.source
            license = $package.license; license_file = $package.license_file; id = $package.id
        }
    }
}

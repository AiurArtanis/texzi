param(
    [Parameter(Mandatory)] [ValidatePattern('^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$')]
    [string] $Tag,
    [Parameter(Mandatory)] [ValidatePattern('^[1-9][0-9]*$')] [string] $RunId,
    [Parameter(Mandatory)] [ValidatePattern('^[1-9][0-9]*$')] [string] $RunAttempt,
    [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{64}$')] [string] $ApprovedZipHash,
    [switch] $VerifyOnly
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-StrictMode -Version Latest

. "$PSScriptRoot/windows-package.ps1"

function Get-VersionRelease {
    param([string] $Repository, [string] $Tag)

    $query = '.[] | select(.tag_name == "{0}") | {{id, draft, prerelease, html_url, body}}' -f $Tag
    $matches = @(gh api "repos/$Repository/releases?per_page=100" --paginate --jq $query |
        ForEach-Object { ConvertFrom-Json -InputObject $_ })
    if ($matches.Count -gt 1) { throw "More than one Release uses tag $Tag." }
    $matches | Select-Object -First 1
}

function Get-MissingReleaseAssets {
    param(
        [string] $Repository,
        [psobject] $Release,
        [bool] $Prerelease,
        [string[]] $Paths,
        [string[]] $IdentityLines
    )

    if (!$Release.draft -or $Release.prerelease -ne $Prerelease) {
        throw 'An existing Release must be an unpublished draft with the expected prerelease status.'
    }
    $bodyLines = ($Release.body -split '\r?\n').Trim()
    foreach ($line in $IdentityLines) {
        $prefix = ($line -split ':', 2)[0] + ':'
        $found = @($bodyLines | Where-Object { $_.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) })
        if ($found.Count -ne 1 -or $found[0] -ne $line) {
            throw "Existing draft notes do not match the approved build identity for $prefix"
        }
    }

    $endpoint = "repos/$Repository/releases/$($Release.id)/assets?per_page=100"
    $assets = @(gh api $endpoint --paginate --jq '.[] | {name, digest, state}' |
        ForEach-Object { ConvertFrom-Json -InputObject $_ })
    $expectedNames = @($Paths | ForEach-Object { Split-Path $_ -Leaf })
    $unexpected = @($assets | Where-Object { $_.name -cnotin $expectedNames })
    if ($unexpected.Count) {
        throw "Existing draft has assets outside the approved package: $($unexpected.name -join ', '). They were preserved."
    }
    foreach ($path in $Paths) {
        $name = Split-Path $path -Leaf
        $matching = @($assets | Where-Object Name -EQ $name)
        if ($matching.Count -eq 0) {
            $path
            continue
        }
        $expectedDigest = 'sha256:' + (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        if ($matching.Count -ne 1 -or $matching[0].name -cne $name -or
            $matching[0].state -ne 'uploaded' -or $matching[0].digest -ne $expectedDigest) {
            throw "Existing draft asset $name has an unverified or mismatched SHA-256. It was preserved."
        }
    }
}

if ($env:GITHUB_REF -ne 'refs/heads/main' -or !$env:GITHUB_REPOSITORY -or
    $env:GITHUB_SHA -notmatch '^[0-9a-f]{40}$') {
    throw 'Run this workflow from main.'
}
$repository = $env:GITHUB_REPOSITORY
$run = gh api "repos/$repository/actions/runs/$RunId/attempts/$RunAttempt" | ConvertFrom-Json
if ($run.path -ne '.github/workflows/windows-ci.yml' -or $run.event -ne 'push' -or
    $run.head_branch -ne 'main' -or $run.conclusion -ne 'success' -or
    $run.head_repository.full_name -ne $repository -or $run.head_sha -notmatch '^[0-9a-f]{40}$') {
    throw 'The selected run attempt must be a successful Windows CI push build from main.'
}
$commit = $run.head_sha

$ref = gh api "repos/$repository/git/ref/tags/$Tag" | ConvertFrom-Json
$tagTarget = $ref.object
while ($tagTarget.type -eq 'tag') {
    $tagTarget = (gh api "repos/$repository/git/tags/$($tagTarget.sha)" | ConvertFrom-Json).object
}
if ($tagTarget.type -ne 'commit' -or $tagTarget.sha -ne $commit) {
    throw 'The existing version tag does not point to the selected CI commit.'
}
$comparison = gh api "repos/$repository/compare/$commit...$env:GITHUB_SHA" | ConvertFrom-Json
if ($comparison.status -notin 'identical', 'ahead' -or $comparison.merge_base_commit.sha -ne $commit) {
    throw 'The selected CI commit is not on main.'
}

$artifact = "texzi-win-x64-$commit-attempt-$RunAttempt"
$download = Join-Path ([IO.Path]::GetTempPath()) ("texzi-release-" + [guid]::NewGuid())
gh run download $RunId --repo $repository --name $artifact --dir $download
$expectedCandidate = @{
    Directory = $download
    Commit = $commit
    RunId = $RunId
    RunAttempt = $RunAttempt
    ApprovedZipHash = $ApprovedZipHash
}
$candidate = Confirm-WindowsCandidate @expectedCandidate
Write-Host "commit=$commit ZIP SHA-256=$($candidate.ZipSha256) EXE SHA-256=$($candidate.ExeSha256)"
if ($VerifyOnly) { return }

$paths = @($candidate.ZipPath, $candidate.ChecksumPath)
$prerelease = $Tag.Contains('-')
$identityLines = @(
    "- Commit: $commit"
    "- ZIP SHA-256: $($candidate.ZipSha256)"
    "- EXE SHA-256: $($candidate.ExeSha256)"
    "- Build: $($run.html_url)/attempts/$RunAttempt"
)
$expectedDraft = @{
    Repository = $repository
    Prerelease = $prerelease
    Paths = $paths
    IdentityLines = $identityLines
}
$draft = Get-VersionRelease -Repository $repository -Tag $Tag
if ($draft) {
    $missing = @(Get-MissingReleaseAssets @expectedDraft -Release $draft)
    if ($missing.Count) {
        gh release upload $Tag @missing --repo $repository
    }
} else {
    $notes = @('Windows x64 便携版。解压后运行 texzi-app.exe。', '') + $identityLines
    $release = @(
        'release', 'create', $Tag, $candidate.ZipPath, $candidate.ChecksumPath,
        '--repo', $repository, '--verify-tag', '--draft', '--latest=false',
        '--title', "texzi $Tag", '--notes', ($notes -join [Environment]::NewLine)
    )
    if ($prerelease) { $release += '--prerelease' }
    gh @release
}

$draft = Get-VersionRelease -Repository $repository -Tag $Tag
if (!$draft) { throw "The Release draft for $Tag could not be read back." }
$missing = @(Get-MissingReleaseAssets @expectedDraft -Release $draft)
if ($missing.Count) { throw 'The Release draft still lacks verified assets. Retry with the same run and hash.' }
Write-Host "Verified draft: $($draft.html_url)"

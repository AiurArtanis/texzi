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

$notes = @"
Windows x64 便携版。解压后运行 texzi-app.exe。

- Commit: $commit
- ZIP SHA-256: $($candidate.ZipSha256)
- EXE SHA-256: $($candidate.ExeSha256)
- Build: $($run.html_url)/attempts/$RunAttempt
"@
$release = @(
    'release', 'create', $Tag, $candidate.ZipPath, $candidate.ChecksumPath,
    '--repo', $repository, '--verify-tag', '--draft', '--latest=false',
    '--title', "texzi $Tag", '--notes', $notes
)
if ($Tag.Contains('-')) { $release += '--prerelease' }
gh @release

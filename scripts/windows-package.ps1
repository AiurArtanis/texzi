$windowsTarget = 'x86_64-pc-windows-msvc'

function Read-CandidateEntry {
    param([IO.Compression.ZipArchive] $Archive, [string] $Name)

    $entry = $Archive.GetEntry($Name)
    if (!$entry) { throw "The candidate ZIP is missing $Name." }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { $reader.ReadToEnd() } finally { $reader.Dispose() }
}

function Confirm-WindowsCandidate {
    param(
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{40}$')] [string] $Commit,
        [Parameter(Mandatory)] [string] $RunId,
        [Parameter(Mandatory)] [string] $RunAttempt,
        [ValidatePattern('^[0-9a-f]{64}$')] [string] $ApprovedZipHash
    )

    $Directory = (Resolve-Path -LiteralPath $Directory).Path
    $zipName = "texzi-$($Commit.Substring(0, 7))-win-x64.zip"
    $zipPath = Join-Path $Directory $zipName
    $checksumPath = Join-Path $Directory 'SHA256SUMS.txt'
    $zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
    $listedZipHash = (Get-Content -LiteralPath $checksumPath -Raw).Trim()
    if ($listedZipHash -ne "$zipHash  $zipName" -or ($ApprovedZipHash -and $ApprovedZipHash -ne $zipHash)) {
        throw 'The candidate ZIP SHA-256 does not match its checksum or the approved hash.'
    }

    $archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $identity = ConvertFrom-StringData (Read-CandidateEntry $archive 'build-identity.txt')
        if ($identity.commit -ne $Commit -or $identity.run -ne $RunId -or
            $identity.attempt -ne $RunAttempt -or $identity.target -ne $windowsTarget) {
            throw 'The candidate build identity does not match the expected commit, run, attempt and target.'
        }
        $entry = $archive.GetEntry('texzi-app.exe')
        if (!$entry) { throw 'The candidate ZIP is missing texzi-app.exe.' }
        $stream = $entry.Open()
        try { $exeHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) }
        finally { $stream.Dispose() }
        if ((Read-CandidateEntry $archive 'SHA256SUMS.txt').Trim() -ne "$exeHash  texzi-app.exe") {
            throw 'The texzi-app.exe SHA-256 does not match the checksum inside the candidate ZIP.'
        }
    } finally {
        $archive.Dispose()
    }

    [pscustomobject]@{
        ZipPath = $zipPath
        ChecksumPath = $checksumPath
        ZipSha256 = $zipHash
        ExeSha256 = $exeHash
    }
}

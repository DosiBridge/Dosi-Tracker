<#
.SYNOPSIS
  Authenticode-sign one file (the agent, the installer or its uninstaller),
  when a signing certificate is configured; otherwise do nothing.

.DESCRIPTION
  Called by dosi-tracker.nsi while it compiles (only when one of the
  variables below is set) and usable on its own. Signs with SHA-256 and an
  RFC 3161 timestamp, so the signature stays valid after the certificate
  expires. The certificate comes from the environment — CI secrets, never
  the repository:

    DOSI_SIGN_PFX            a .pfx file (with DOSI_SIGN_PFX_PASSWORD), or
    DOSI_SIGN_CERT_SHA1      the thumbprint of a certificate in the user's or
                             machine's store (a hardware token, a cloud HSM), or
    DOSI_SIGN_DLIB           a signtool dlib such as Azure Trusted Signing's
                             Azure.CodeSigning.Dlib.dll (with DOSI_SIGN_DMDF,
                             its metadata.json)

    DOSI_SIGN_TIMESTAMP_URL  the RFC 3161 timestamp server
                             (default http://timestamp.digicert.com)
    DOSI_SIGNTOOL            signtool.exe (default: the newest Windows SDK's)

  With none of the three set, the file is left unsigned and this exits 0.
  A configuration that is set but unusable fails (exit 1): a release meant
  to be signed must never ship unsigned by accident.
#>
param(
    [Parameter(Mandatory = $true)][string]$Path
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    [Console]::Error.WriteLine("sign.ps1: $Message")
    exit 1
}

$pfx = $env:DOSI_SIGN_PFX
$thumbprint = $env:DOSI_SIGN_CERT_SHA1
$dlib = $env:DOSI_SIGN_DLIB
if (-not ($pfx -or $thumbprint -or $dlib)) {
    Write-Host "sign.ps1: not signing $Path (no signing certificate configured)"
    exit 0
}
if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    Fail "no file to sign at $Path"
}

$signtool = $env:DOSI_SIGNTOOL
if (-not $signtool) {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) {
        $signtool = $onPath.Source
    } else {
        $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
        $found = Get-ChildItem -Path $kits -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.Directory.Name -eq 'x64' } |
            Sort-Object { $_.Directory.Parent.Name } -Descending |
            Select-Object -First 1
        if ($found) { $signtool = $found.FullName }
    }
}
if (-not $signtool -or -not (Test-Path -LiteralPath $signtool)) {
    Fail 'signtool.exe not found: install the Windows SDK or set DOSI_SIGNTOOL'
}

$timestamp = $env:DOSI_SIGN_TIMESTAMP_URL
if (-not $timestamp) { $timestamp = 'http://timestamp.digicert.com' }

$arguments = @('sign', '/fd', 'SHA256', '/tr', $timestamp, '/td', 'SHA256',
    '/d', 'Dosi Tracker', '/du', 'https://github.com/DosiBridge/Dosi-Tracker')
$shown = $arguments.Clone()
if ($dlib) {
    $metadata = $env:DOSI_SIGN_DMDF
    if (-not $metadata -or -not (Test-Path -LiteralPath $metadata)) {
        Fail 'DOSI_SIGN_DLIB needs DOSI_SIGN_DMDF, the path of its metadata file'
    }
    $arguments += @('/dlib', $dlib, '/dmdf', $metadata)
    $shown += @('/dlib', $dlib, '/dmdf', $metadata)
} elseif ($thumbprint) {
    $arguments += @('/sha1', $thumbprint)
    $shown += @('/sha1', $thumbprint)
} else {
    if (-not (Test-Path -LiteralPath $pfx -PathType Leaf)) {
        Fail "DOSI_SIGN_PFX names no file ($pfx)"
    }
    $arguments += @('/f', $pfx)
    $shown += @('/f', $pfx)
    if ($env:DOSI_SIGN_PFX_PASSWORD) {
        # The password goes to signtool only; it is never printed.
        $arguments += @('/p', $env:DOSI_SIGN_PFX_PASSWORD)
        $shown += @('/p', '***')
    }
}

Write-Host "sign.ps1: $signtool $($shown -join ' ') $Path"
& $signtool @arguments $Path
if ($LASTEXITCODE -ne 0) {
    Fail "signtool could not sign $Path (exit $LASTEXITCODE)"
}

$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -eq 'NotSigned' -or -not $signature.SignerCertificate) {
    Fail "$Path is still not signed"
}
if (-not $signature.TimeStamperCertificate) {
    Fail "$Path was signed without a timestamp"
}
if ($signature.Status -ne 'Valid') {
    # A test certificate this machine does not trust signs fine but does not
    # verify; a release certificate always should.
    Write-Warning "sign.ps1: $Path is signed but does not verify here ($($signature.Status): $($signature.StatusMessage))"
}
Write-Host "sign.ps1: signed $Path ($($signature.SignerCertificate.Subject))"
exit 0

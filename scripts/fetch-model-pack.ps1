# Builds an offline ORAG model pack on a connected Windows machine (D-012),
# the PowerShell twin of scripts/fetch-model-pack.sh: the same presets,
# revisions and SHA-256 hashes (scripts/check-pack-presets.sh compares them).
# Usage: powershell -ExecutionPolicy Bypass -File fetch-model-pack.ps1 [-Preset <id>] [-OutDir <folder>]
# Without -Preset it builds the two default packs. Then import the folder from
# the ORAG page (Modelleri kur) or with: orag.exe models import <folder>\<preset>
param(
  [string]$Preset = '',
  [string]$OutDir = (Join-Path $env:USERPROFILE 'ORAG-paketleri'),
  [switch]$Yes
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Windows PowerShell 5.1 (the Start-menu shortcut) may default to TLS 1.0;
# huggingface.co needs TLS 1.2 or newer.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
# .NET file calls resolve relative paths against the process directory, not
# the PowerShell location: make the output folder absolute once.
if (-not [IO.Path]::IsPathRooted($OutDir)) {
  $OutDir = Join-Path (Get-Location).ProviderPath $OutDir
}
$OutDir = [IO.Path]::GetFullPath($OutDir)

$Presets = [ordered]@{
  'qwen3-embedding-0.6b-q8_0' = @{
    repo = 'Qwen/Qwen3-Embedding-0.6B-GGUF'
    revision = '370f27d7550e0def9b39c1f16d3fbaa13aa67728'
    file = 'Qwen3-Embedding-0.6B-Q8_0.gguf'
    sha = '06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439'
    license_url = 'https://www.apache.org/licenses/LICENSE-2.0.txt'
    license_sha = 'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30'
    role = 'embedding'
    role_section = '[embedding]`ndimensions = 1024`npooling = "last"`nmax_tokens = 1024`nquery_prefix = "Instruct: Given a search query, retrieve relevant passages that answer the query\nQuery: "`ndocument_prefix = ""`nrequire_trailing_eos = true'
  }
  'qwen3.5-4b-q4_k_m' = @{
    repo = 'unsloth/Qwen3.5-4B-GGUF'
    revision = 'e87f176479d0855a907a41277aca2f8ee7a09523'
    file = 'Qwen3.5-4B-Q4_K_M.gguf'
    sha = '00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4'
    license_url = 'https://huggingface.co/Qwen/Qwen3.5-4B/resolve/851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a/LICENSE'
    license_sha = 'bbedc3fda3305820b977265f01b8619d87570a6739de3a5582c3464840f1e57a'
    role = 'generation'
    role_section = '[generation]`ncontext_tokens = 8192`nmax_output_tokens = 1024`nprompt_format = "chatml-nothink"'
  }
  'qwen3-4b-instruct-2507-q4_k_m' = @{
    repo = 'unsloth/Qwen3-4B-Instruct-2507-GGUF'
    revision = 'a06e946bb6b655725eafa393f4a9745d460374c9'
    file = 'Qwen3-4B-Instruct-2507-Q4_K_M.gguf'
    sha = '3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597'
    license_url = 'https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/resolve/cdbee75f17c01a7cc42f958dc650907174af0554/LICENSE'
    license_sha = '832dd9e00a68dd83b3c3fb9f5588dad7dcf337a0db50f7d9483f310cd292e92e'
    role = 'generation'
    role_section = '[generation]`ncontext_tokens = 8192`nmax_output_tokens = 1024`nprompt_format = "chatml"'
  }
  'bge-m3-q8_0' = @{
    repo = 'ggml-org/bge-m3-Q8_0-GGUF'
    revision = '9eba04c5d75ba5a1595e45de734d36bef4e5cb98'
    file = 'bge-m3-q8_0.gguf'
    sha = 'aa473d51f451a22f0fcf39ba3330c14bed38a385712b1113440f69df4047a173'
    card_url = 'https://huggingface.co/BAAI/bge-m3/resolve/5617a9f61b028005a4858fdac845db406aefb181/README.md'
    card_sha = '0b81ccf9134e5874d620a86e6905062ea999e779c34eb1a7e65eaeb7fe00e450'
    role = 'embedding'
    role_section = '[embedding]`ndimensions = 1024`npooling = "cls"`nmax_tokens = 1024`nquery_prefix = ""`ndocument_prefix = ""'
    license_name = 'MIT'
    mit_holder = 'the authors of BAAI/bge-m3'
  }
  'multilingual-e5-large-instruct-q8_0' = @{
    repo = 'Ralriki/multilingual-e5-large-instruct-GGUF'
    revision = 'f43c843a00e7c3bdd087cbaf481464c1fa7fb37d'
    file = 'multilingual-e5-large-instruct-q8_0.gguf'
    sha = '2fa4f519b83ecd31a35265b9dd43790bf0abd1ff942e7b982861ee674dc7bec8'
    card_url = 'https://huggingface.co/intfloat/multilingual-e5-large-instruct/resolve/274baa43b0e13e37fafa6428dbc7938e62e5c439/README.md'
    card_sha = '41676700d80691ac2f4397ee2b546ef58f9e3236af2f3d0770ab0fa61940ce51'
    role = 'embedding'
    role_section = '[embedding]`ndimensions = 1024`npooling = "mean"`nmax_tokens = 512`nquery_prefix = "Instruct: Given a search query, retrieve relevant passages that answer the query\nQuery: "`ndocument_prefix = ""'
    license_name = 'MIT'
    mit_holder = 'the authors of intfloat/multilingual-e5-large-instruct'
  }
}
$Defaults = @('qwen3-embedding-0.6b-q8_0', 'qwen3.5-4b-q4_k_m')

$MitText = @'
MIT License

Copyright (c) {0}
(declared as "license: mit" in MODEL_CARD.md, revision {1} of the GGUF
conversion; see https://huggingface.co/{2})

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
'@

function Get-Sha256([string]$Path) {
  (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# Target ends up with exactly the pinned bytes or not at all. A .part is
# resumed with a Range request; one that does not verify is deleted.
function Get-Pinned([string]$Url, [string]$Expected, [string]$Target) {
  if ((Test-Path -LiteralPath $Target) -and ((Get-Sha256 $Target) -eq $Expected)) { return }
  Remove-Item -LiteralPath $Target -ErrorAction SilentlyContinue
  $part = "$Target.part"
  Write-Host "indiriliyor / downloading $Url"
  $resumed = $false
  if (Test-Path -LiteralPath $part) {
    $have = (Get-Item -LiteralPath $part).Length
    $req = [System.Net.HttpWebRequest]::Create($Url)
    $req.AddRange([long]$have)
    try {
      $resp = $req.GetResponse()
    } catch [System.Net.WebException] {
      $resp = $_.Exception.Response
      # 416: the .part already holds the whole file; check it as it is.
      if ($resp -and [int]$resp.StatusCode -eq 416) { $resp.Dispose(); $resumed = $true }
      elseif ($resp) { $resp.Dispose(); $resp = $null }
      else { throw }
    }
    if ($resp) {
      if ([int]$resp.StatusCode -eq 206) {
        # A failure mid-copy keeps what arrived: run the script again to go on.
        $in = $resp.GetResponseStream()
        $out = [System.IO.File]::Open($part, 'Append', 'Write')
        try { $in.CopyTo($out) } finally { $out.Dispose(); $in.Dispose(); $resp.Dispose() }
        $resumed = $true
      } else {
        $resp.Dispose()
      }
    }
  }
  if (-not $resumed) {
    # No .part, or the server ignored the range: download from the start.
    Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue
    Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing
  }
  $actual = Get-Sha256 $part
  if ($actual -ne $Expected) {
    Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue
    throw "checksum mismatch for $(Split-Path -Leaf $Target): expected $Expected, got $actual (removed; run again)"
  }
  Move-Item -LiteralPath $part -Destination $Target -Force
}

function New-Pack([string]$Id) {
  if (-not $Presets.Contains($Id)) { throw "unknown preset: $Id (known: $($Presets.Keys -join ', '))" }
  $p = $Presets[$Id]
  $dest = Join-Path $OutDir $Id
  New-Item -ItemType Directory -Force -Path $dest | Out-Null
  # The manifest is written last, so a pack without one was never finished.
  Remove-Item -LiteralPath (Join-Path $dest 'orag-model.toml') -ErrorAction SilentlyContinue
  Get-Pinned "https://huggingface.co/$($p.repo)/resolve/$($p.revision)/$($p.file)" $p.sha (Join-Path $dest $p.file)
  $license = if ($p.Contains('license_name')) { $p.license_name } else { 'Apache-2.0' }
  if ($p.Contains('mit_holder')) {
    Get-Pinned $p.card_url $p.card_sha (Join-Path $dest 'MODEL_CARD.md')
    $mit = $MitText -f $p.mit_holder, $p.revision, $p.repo
    [System.IO.File]::WriteAllText((Join-Path $dest 'LICENSE'), ($mit -replace "`r`n", "`n") + "`n")
  } else {
    Get-Pinned $p.license_url $p.license_sha (Join-Path $dest 'LICENSE')
  }
  $section = $p.role_section -replace '`n', "`n"
  $manifest = @(
    "id = `"$Id`"",
    "role = `"$($p.role)`"",
    "file = `"$($p.file)`"",
    "sha256 = `"$($p.sha)`"",
    "license = `"$license`"",
    'license_file = "LICENSE"',
    "source = `"https://huggingface.co/$($p.repo)`"",
    "revision = `"$($p.revision)`"",
    '',
    $section
  ) -join "`n"
  [System.IO.File]::WriteAllText((Join-Path $dest 'orag-model.toml'), $manifest + "`n")
  Write-Host "paket hazir / pack ready: $dest"
}

$ids = if ($Preset) { @($Preset) } else { $Defaults }
if (-not $Yes) {
  Write-Host 'ORAG modelleri internetten (huggingface.co) indirilecek: yaklasik 3,3 GB.'
  Write-Host "Hedef klasor: $OutDir"
  Write-Host 'This downloads the ORAG models from huggingface.co (about 3.3 GB).'
  $answer = Read-Host 'Devam edilsin mi? / Continue? [e/E = evet, y/Y = yes]'
  if ($answer -notmatch '^[eEyY]') { Write-Host 'Iptal edildi. / Cancelled.'; exit 1 }
}
foreach ($id in $ids) { New-Pack $id }
Write-Host ''
Write-Host "Bitti. ORAG sayfasinda 'Modelleri kur' alanina bu klasoru yazin:"
Write-Host "Done. In the ORAG page, type this folder into 'Modelleri kur':"
Write-Host "  $OutDir"

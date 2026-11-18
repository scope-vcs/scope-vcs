export type CliRelease = {
  tag: string
  artifacts: Record<string, string>
}

export type CliDistributionTarget = {
  artifact: string
  os: string
  arch: string
}

const originPattern = /^https?:\/\/[A-Za-z0-9.-]+(:[0-9]+)?$/
const tagPattern = /^cli-[A-Za-z0-9.-]{1,64}$/
const artifactPattern = /^[A-Za-z0-9._-]+$/
const checksumPattern = /^[a-f0-9]{64}$/

const posixPatterns: Record<string, string> = {
  'linux-x64': 'Linux-x86_64|Linux-amd64',
  'linux-arm64': 'Linux-aarch64|Linux-arm64',
  'macos-x64': 'Darwin-x86_64|Darwin-amd64',
  'macos-arm64': 'Darwin-arm64|Darwin-aarch64',
}

const windowsArchitectures: Record<string, string> = {
  'windows-x64': 'X64',
  'windows-arm64': 'Arm64',
}

export function posixInstallScript(
  apiOrigin: string,
  release: CliRelease,
  targets: readonly CliDistributionTarget[],
) {
  const cases = installable(release, targets, posixPatterns)
    .map(
      ({ pattern, artifact, checksum }) =>
        `  ${pattern})\n    artifact="${artifact}"\n    expected="${checksum}"\n    ;;\n`,
    )
    .join('\n')
  return posixTemplate
    .replace('{download_url}', () => downloadUrl(apiOrigin, release))
    .replace('{cases}', () => cases)
}

export function windowsInstallScript(
  apiOrigin: string,
  release: CliRelease,
  targets: readonly CliDistributionTarget[],
) {
  const cases = installable(release, targets, windowsArchitectures)
    .map(
      ({ pattern, artifact, checksum }) =>
        `  "${pattern}" {\n    $artifact = "${artifact}"\n    $expected = "${checksum}"\n  }\n`,
    )
    .join('\n')
  return windowsTemplate
    .replace('{download_url}', () => downloadUrl(apiOrigin, release))
    .replace('{cases}', () => cases)
}

function downloadUrl(apiOrigin: string, release: CliRelease) {
  if (!originPattern.test(apiOrigin)) {
    throw new Error('The CLI download origin must be an HTTP(S) origin')
  }
  if (!tagPattern.test(release.tag)) {
    throw new Error('The CLI release tag is not a published CLI release')
  }
  return `${apiOrigin}/cli/downloads/${release.tag}`
}

function installable(
  release: CliRelease,
  targets: readonly CliDistributionTarget[],
  patterns: Record<string, string>,
) {
  return targets.flatMap((target) => {
    const pattern = patterns[`${target.os}-${target.arch}`]
    if (!pattern) return []
    const checksum = release.artifacts[target.artifact]
    if (!artifactPattern.test(target.artifact) || !checksumPattern.test(checksum ?? '')) {
      throw new Error(`The CLI release has no checksum for ${target.artifact}`)
    }
    return [{ pattern, artifact: target.artifact, checksum }]
  })
}

const posixTemplate = `#!/bin/sh
set -eu

download_url='{download_url}'
target="$(uname -s)-$(uname -m)"

case "$target" in
{cases}
  *)
    echo "scope installer does not have a binary for $target yet." >&2
    exit 1
    ;;
esac

path_contains() {
  case ":\${PATH:-}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

can_sudo() {
  [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1
}

select_install_dir() {
  if [ -n "\${SCOPE_INSTALL_DIR:-}" ]; then
    install_dir="$SCOPE_INSTALL_DIR"
    if path_contains "$install_dir" && { [ ! -d "$install_dir" ] || [ ! -w "$install_dir" ]; } && can_sudo; then
      install_with_sudo=1
    fi
    return
  fi

  existing="$(command -v scope 2>/dev/null || true)"
  case "$existing" in
    /*)
      candidate="\${existing%/*}"
      if [ -w "$candidate" ] && path_contains "$candidate"; then
        install_dir="$candidate"
        return
      fi
      ;;
  esac

  for candidate in "$HOME/.local/bin" "$HOME/bin" "$HOME/.cargo/bin" "/opt/homebrew/bin" "/usr/local/bin"; do
    if [ -d "$candidate" ] && [ -w "$candidate" ] && path_contains "$candidate"; then
      install_dir="$candidate"
      return
    fi
  done

  old_ifs="$IFS"
  IFS=:
  for candidate in \${PATH:-}; do
    IFS="$old_ifs"
    case "$candidate" in
      /*)
        if [ -d "$candidate" ] && [ -w "$candidate" ]; then
          install_dir="$candidate"
          return
        fi
        ;;
    esac
    IFS=:
  done
  IFS="$old_ifs"

  if can_sudo; then
    for candidate in "/usr/local/bin" "/opt/homebrew/bin"; do
      if [ -d "$candidate" ] && path_contains "$candidate"; then
        install_dir="$candidate"
        install_with_sudo=1
        return
      fi
    done

    old_ifs="$IFS"
    IFS=:
    for candidate in \${PATH:-}; do
      IFS="$old_ifs"
      case "$candidate" in
        /*)
          if [ -d "$candidate" ]; then
            install_dir="$candidate"
            install_with_sudo=1
            return
          fi
          ;;
      esac
      IFS=:
    done
    IFS="$old_ifs"
  fi
}

install_dir=""
install_with_sudo=0
select_install_dir
if [ -z "$install_dir" ]; then
  echo "scope installer could not find a writable directory on PATH." >&2
  echo "Add a user bin directory to PATH or rerun with SCOPE_INSTALL_DIR set to a writable PATH directory." >&2
  exit 1
fi

if ! path_contains "$install_dir"; then
  echo "scope install directory is not on PATH: $install_dir" >&2
  echo "Set SCOPE_INSTALL_DIR to a writable directory already on PATH and rerun." >&2
  exit 1
fi

if [ "$install_with_sudo" = 1 ]; then
  sudo mkdir -p "$install_dir"
else
  mkdir -p "$install_dir"
fi

tmp_archive="$(mktemp)"
stage_dir="$(mktemp -d)"
binary_backup="$install_dir/.scope.backup.$$"
binary_backed_up=0
binary_installed=0
committed=0

as_installer() {
  if [ "$install_with_sudo" = 1 ]; then sudo "$@"; else "$@"; fi
}

cleanup() {
  if [ "$committed" = 0 ]; then
    [ "$binary_installed" = 0 ] || as_installer rm -f "$install_dir/scope"
    [ "$binary_backed_up" = 0 ] || as_installer mv "$binary_backup" "$install_dir/scope"
  fi
  rm -rf "$tmp_archive" "$stage_dir"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

curl -fsSL "$download_url/\${artifact}?installer=sh" -o "$tmp_archive"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp_archive" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$tmp_archive" | awk '{print $1}')"
fi

if [ "$expected" != "$actual" ]; then
  echo "scope checksum verification failed for $artifact." >&2
  exit 1
fi

tar -xzf "$tmp_archive" -C "$stage_dir"
test -f "$stage_dir/scope"
chmod 755 "$stage_dir/scope"

as_installer rm -f "$binary_backup"
if as_installer test -e "$install_dir/scope"; then
  as_installer mv "$install_dir/scope" "$binary_backup"
  binary_backed_up=1
fi
as_installer mv "$stage_dir/scope" "$install_dir/scope"
binary_installed=1
committed=1
as_installer rm -f "$binary_backup"
echo "scope installed to $install_dir/scope"

resolved="$(command -v scope 2>/dev/null || true)"
if [ -z "$resolved" ] || ! [ "$resolved" -ef "$install_dir/scope" ]; then
  echo "scope at $resolved comes first on PATH, so the shell will not run the version just installed." >&2
  echo "Remove it, or rerun with SCOPE_INSTALL_DIR=\${resolved%/*} to update it." >&2
  exit 1
fi
`

const windowsTemplate = `$ErrorActionPreference = "Stop"

$downloadUrl = '{download_url}'
$installDir = if ($env:SCOPE_INSTALL_DIR) { $env:SCOPE_INSTALL_DIR } else { Join-Path $HOME ".local\\bin" }
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()

switch ($arch) {
{cases}
  default {
    throw "scope installer does not have a Windows binary for $arch yet."
  }
}

function ConvertTo-ComparablePath([string] $path) {
  try {
    return [System.IO.Path]::GetFullPath($path).TrimEnd([char[]]@('\\', '/')).ToLowerInvariant()
  } catch {
    return $path.TrimEnd([char[]]@('\\', '/')).ToLowerInvariant()
  }
}

function Test-PathListContains([string] $pathList, [string] $directory) {
  if ([string]::IsNullOrWhiteSpace($pathList)) {
    return $false
  }

  $needle = ConvertTo-ComparablePath $directory
  foreach ($entry in $pathList -split [System.Text.RegularExpressions.Regex]::Escape([System.IO.Path]::PathSeparator)) {
    if ([string]::IsNullOrWhiteSpace($entry)) {
      continue
    }

    if ((ConvertTo-ComparablePath $entry) -eq $needle) {
      return $true
    }
  }

  return $false
}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$tmpFile = New-TemporaryFile
$tmpPath = $tmpFile.FullName
$stagePath = Join-Path $installDir (".scope-stage-" + [guid]::NewGuid().ToString("N"))
$destination = Join-Path $installDir "scope.exe"
$binaryBackup = Join-Path $installDir (".scope.backup-" + [guid]::NewGuid().ToString("N"))
$binaryBackedUp = $false
$binaryInstalled = $false
$committed = $false

try {
  Invoke-WebRequest -Uri "$downloadUrl/\${artifact}?installer=ps1" -OutFile $tmpPath

  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmpPath).Hash.ToLowerInvariant()
  if ($expected -ne $actual) {
    throw "scope checksum verification failed for $artifact."
  }

  New-Item -ItemType Directory -Path $stagePath | Out-Null
  & tar.exe -xzf $tmpPath -C $stagePath
  if ($LASTEXITCODE -ne 0) { throw "scope bundle extraction failed for $artifact." }
  $stagedBinary = Join-Path $stagePath "scope.exe"
  if (-not (Test-Path -LiteralPath $stagedBinary)) { throw "scope bundle is missing $stagedBinary." }

  if (Test-Path -LiteralPath $destination) {
    Move-Item -LiteralPath $destination -Destination $binaryBackup
    $binaryBackedUp = $true
  }
  Move-Item -LiteralPath $stagedBinary -Destination $destination
  $binaryInstalled = $true
  $committed = $true
  Remove-Item -LiteralPath $binaryBackup -Force -ErrorAction SilentlyContinue
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
  if (
    -not (Test-PathListContains $userPath $installDir) -and
    -not (Test-PathListContains $machinePath $installDir)
  ) {
    $separator = [System.IO.Path]::PathSeparator
    $nextUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
      $installDir
    } else {
      "$userPath$separator$installDir"
    }
    [Environment]::SetEnvironmentVariable("Path", $nextUserPath, "User")
  }

  if (-not (Test-PathListContains $env:Path $installDir)) {
    $separator = [System.IO.Path]::PathSeparator
    $env:Path = if ([string]::IsNullOrWhiteSpace($env:Path)) {
      $installDir
    } else {
      "$env:Path$separator$installDir"
    }
  }

  Write-Output "scope installed to $destination"
} finally {
  if (-not $committed) {
    if ($binaryInstalled) { Remove-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue }
    if ($binaryBackedUp) { Move-Item -LiteralPath $binaryBackup -Destination $destination -Force }
  }
  Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $stagePath -Recurse -Force -ErrorAction SilentlyContinue
}
`

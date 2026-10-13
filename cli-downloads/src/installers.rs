use crate::distribution::{DistributionManifest, DistributionTarget};

pub fn posix_install_script(base_url: &str, manifest: &DistributionManifest) -> String {
    let base_url = format!(
        "'{}'",
        base_url.trim_end_matches('/').replace('\'', "'\"'\"'")
    );
    let cases = manifest
        .targets()
        .iter()
        .filter_map(posix_case_arm)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"#!/bin/sh
set -eu

base_url={base_url}
target="$(uname -s)-$(uname -m)"

case "$target" in
{cases}
  *)
    echo "scope installer does not have a binary for $target yet." >&2
    exit 1
    ;;
esac

path_contains() {{
  case ":${{PATH:-}}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}}

can_sudo() {{
  [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1
}}

select_install_dir() {{
  if [ -n "${{SCOPE_INSTALL_DIR:-}}" ]; then
    install_dir="$SCOPE_INSTALL_DIR"
    if path_contains "$install_dir" && {{ [ ! -d "$install_dir" ] || [ ! -w "$install_dir" ]; }} && can_sudo; then
      install_with_sudo=1
    fi
    return
  fi

  for candidate in "$HOME/.local/bin" "$HOME/bin" "$HOME/.cargo/bin" "/opt/homebrew/bin" "/usr/local/bin"; do
    if [ -d "$candidate" ] && [ -w "$candidate" ] && path_contains "$candidate"; then
      install_dir="$candidate"
      return
    fi
  done

  old_ifs="$IFS"
  IFS=:
  for candidate in ${{PATH:-}}; do
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
    for candidate in ${{PATH:-}}; do
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
}}

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
checksum_file="$(mktemp)"
stage_dir="$(mktemp -d)"
runtime_backup="$install_dir/.scope-runtime.backup.$$"
binary_backup="$install_dir/.scope.backup.$$"
runtime_backed_up=0
runtime_installed=0
binary_backed_up=0
binary_installed=0
committed=0

as_installer() {{
  if [ "$install_with_sudo" = 1 ]; then sudo "$@"; else "$@"; fi
}}

cleanup() {{
  if [ "$committed" = 0 ]; then
    [ "$binary_installed" = 0 ] || as_installer rm -f "$install_dir/scope"
    [ "$binary_backed_up" = 0 ] || as_installer mv "$binary_backup" "$install_dir/scope"
    [ "$runtime_installed" = 0 ] || as_installer rm -rf "$install_dir/scope-runtime"
    [ "$runtime_backed_up" = 0 ] || as_installer mv "$runtime_backup" "$install_dir/scope-runtime"
  fi
  rm -rf "$tmp_archive" "$checksum_file" "$stage_dir"
}}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

curl -fsSL "$base_url/downloads/$artifact" -o "$tmp_archive"
curl -fsSL "$base_url/downloads/$artifact.sha256" -o "$checksum_file"

expected="$(awk '{{print $1}}' "$checksum_file")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp_archive" | awk '{{print $1}}')"
else
  actual="$(shasum -a 256 "$tmp_archive" | awk '{{print $1}}')"
fi

if [ "$expected" != "$actual" ]; then
  echo "scope checksum verification failed for $artifact." >&2
  exit 1
fi

tar -xzf "$tmp_archive" -C "$stage_dir"
test -f "$stage_dir/scope"
test -f "$stage_dir/scope-runtime/node"
test -f "$stage_dir/scope-runtime/dependency-analyzer/analyze.mjs"
test -d "$stage_dir/scope-runtime/dependency-analyzer/node_modules"
chmod 755 "$stage_dir/scope" "$stage_dir/scope-runtime/node"

as_installer rm -rf "$runtime_backup" "$binary_backup"
if as_installer test -e "$install_dir/scope-runtime"; then
  as_installer mv "$install_dir/scope-runtime" "$runtime_backup"
  runtime_backed_up=1
fi
as_installer mv "$stage_dir/scope-runtime" "$install_dir/scope-runtime"
runtime_installed=1
if as_installer test -e "$install_dir/scope"; then
  as_installer mv "$install_dir/scope" "$binary_backup"
  binary_backed_up=1
fi
as_installer mv "$stage_dir/scope" "$install_dir/scope"
binary_installed=1
committed=1
as_installer rm -rf "$runtime_backup" "$binary_backup"
echo "scope installed to $install_dir/scope"
"#,
    )
}

pub fn windows_install_script(base_url: &str, manifest: &DistributionManifest) -> String {
    let base_url = format!("'{}'", base_url.trim_end_matches('/').replace('\'', "''"));
    let cases = manifest
        .targets()
        .iter()
        .filter_map(windows_case_arm)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"$ErrorActionPreference = "Stop"

$baseUrl = {base_url}
$installDir = if ($env:SCOPE_INSTALL_DIR) {{ $env:SCOPE_INSTALL_DIR }} else {{ Join-Path $HOME ".local\bin" }}
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()

switch ($arch) {{
{cases}
  default {{
    throw "scope installer does not have a Windows binary for $arch yet."
  }}
}}

function ConvertTo-ComparablePath([string] $path) {{
  try {{
    return [System.IO.Path]::GetFullPath($path).TrimEnd([char[]]@('\', '/')).ToLowerInvariant()
  }} catch {{
    return $path.TrimEnd([char[]]@('\', '/')).ToLowerInvariant()
  }}
}}

function Test-PathListContains([string] $pathList, [string] $directory) {{
  if ([string]::IsNullOrWhiteSpace($pathList)) {{
    return $false
  }}

  $needle = ConvertTo-ComparablePath $directory
  foreach ($entry in $pathList -split [System.Text.RegularExpressions.Regex]::Escape([System.IO.Path]::PathSeparator)) {{
    if ([string]::IsNullOrWhiteSpace($entry)) {{
      continue
    }}

    if ((ConvertTo-ComparablePath $entry) -eq $needle) {{
      return $true
    }}
  }}

  return $false
}}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$tmpFile = New-TemporaryFile
$checksumFile = New-TemporaryFile
$tmpPath = $tmpFile.FullName
$checksumPath = $checksumFile.FullName
$stagePath = Join-Path $installDir (".scope-stage-" + [guid]::NewGuid().ToString("N"))
$runtimeDestination = Join-Path $installDir "scope-runtime"
$runtimeBackup = Join-Path $installDir (".scope-runtime.backup-" + [guid]::NewGuid().ToString("N"))
$destination = Join-Path $installDir "scope.exe"
$binaryBackup = Join-Path $installDir (".scope.backup-" + [guid]::NewGuid().ToString("N"))
$runtimeBackedUp = $false
$runtimeInstalled = $false
$binaryBackedUp = $false
$binaryInstalled = $false
$committed = $false

try {{
  Invoke-WebRequest -Uri "$baseUrl/downloads/$artifact" -OutFile $tmpPath
  Invoke-WebRequest -Uri "$baseUrl/downloads/$artifact.sha256" -OutFile $checksumPath

  $expected = ((Get-Content -LiteralPath $checksumPath | Select-Object -First 1) -split "\s+")[0].ToLowerInvariant()
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmpPath).Hash.ToLowerInvariant()
  if ($expected -ne $actual) {{
    throw "scope checksum verification failed for $artifact."
  }}

  New-Item -ItemType Directory -Path $stagePath | Out-Null
  & tar.exe -xzf $tmpPath -C $stagePath
  if ($LASTEXITCODE -ne 0) {{ throw "scope bundle extraction failed for $artifact." }}
  $stagedBinary = Join-Path $stagePath "scope.exe"
  $stagedRuntime = Join-Path $stagePath "scope-runtime"
  foreach ($required in @(
    $stagedBinary,
    (Join-Path $stagedRuntime "node.exe"),
    (Join-Path $stagedRuntime "dependency-analyzer\analyze.mjs"),
    (Join-Path $stagedRuntime "dependency-analyzer\node_modules")
  )) {{
    if (-not (Test-Path -LiteralPath $required)) {{ throw "scope bundle is missing $required." }}
  }}

  if (Test-Path -LiteralPath $runtimeDestination) {{
    Move-Item -LiteralPath $runtimeDestination -Destination $runtimeBackup
    $runtimeBackedUp = $true
  }}
  Move-Item -LiteralPath $stagedRuntime -Destination $runtimeDestination
  $runtimeInstalled = $true
  if (Test-Path -LiteralPath $destination) {{
    Move-Item -LiteralPath $destination -Destination $binaryBackup
    $binaryBackedUp = $true
  }}
  Move-Item -LiteralPath $stagedBinary -Destination $destination
  $binaryInstalled = $true
  $committed = $true
  Remove-Item -LiteralPath $runtimeBackup, $binaryBackup -Recurse -Force -ErrorAction SilentlyContinue
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
  if (
    -not (Test-PathListContains $userPath $installDir) -and
    -not (Test-PathListContains $machinePath $installDir)
  ) {{
    $separator = [System.IO.Path]::PathSeparator
    $nextUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {{
      $installDir
    }} else {{
      "$userPath$separator$installDir"
    }}
    [Environment]::SetEnvironmentVariable("Path", $nextUserPath, "User")
  }}

  if (-not (Test-PathListContains $env:Path $installDir)) {{
    $separator = [System.IO.Path]::PathSeparator
    $env:Path = if ([string]::IsNullOrWhiteSpace($env:Path)) {{
      $installDir
    }} else {{
      "$env:Path$separator$installDir"
    }}
  }}

  Write-Output "scope installed to $destination"
}} finally {{
  if (-not $committed) {{
    if ($binaryInstalled) {{ Remove-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue }}
    if ($binaryBackedUp) {{ Move-Item -LiteralPath $binaryBackup -Destination $destination -Force }}
    if ($runtimeInstalled) {{ Remove-Item -LiteralPath $runtimeDestination -Recurse -Force -ErrorAction SilentlyContinue }}
    if ($runtimeBackedUp) {{ Move-Item -LiteralPath $runtimeBackup -Destination $runtimeDestination -Force }}
  }}
  Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $checksumPath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $stagePath -Recurse -Force -ErrorAction SilentlyContinue
}}
"#,
    )
}

fn posix_case_arm(target: &DistributionTarget) -> Option<String> {
    let pattern = match (target.os.as_str(), target.arch.as_str()) {
        ("linux", "x64") => "  Linux-x86_64|Linux-amd64)",
        ("linux", "arm64") => "  Linux-aarch64|Linux-arm64)",
        ("macos", "x64") => "  Darwin-x86_64|Darwin-amd64)",
        ("macos", "arm64") => "  Darwin-arm64|Darwin-aarch64)",
        _ => return None,
    };

    Some(format!(
        r#"{pattern}
    artifact="{artifact}"
    ;;
"#,
        artifact = target.artifact
    ))
}

fn windows_case_arm(target: &DistributionTarget) -> Option<String> {
    match (target.os.as_str(), target.arch.as_str()) {
        ("windows", "x64") => Some(format!(
            r#"  "X64" {{
    $artifact = "{artifact}"
  }}
"#,
            artifact = target.artifact
        )),
        ("windows", "arm64") => Some(format!(
            r#"  "Arm64" {{
    $artifact = "{artifact}"
  }}
"#,
            artifact = target.artifact
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const ORIGINS: [&str; 4] = [
        "https://example.test/$(printf injected)",
        "https://example.test/`printf injected`",
        "https://example.test/'\"; printf injected; #",
        "https://example.test/$env:HOME",
    ];

    #[cfg(unix)]
    #[test]
    fn posix_origin_assignment_is_literal_data() {
        for origin in ORIGINS {
            let script = posix_install_script(origin, DistributionManifest::bundled());
            let assignment = script
                .lines()
                .find(|line| line.starts_with("base_url="))
                .unwrap();
            let output = Command::new("sh")
                .args(["-c", &format!("{assignment}\nprintf '%s' \"$base_url\"")])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(String::from_utf8(output.stdout).unwrap(), origin);
        }
    }

    #[cfg(windows)]
    #[test]
    fn powershell_origin_assignment_is_literal_data() {
        for origin in ORIGINS {
            let script = windows_install_script(origin, DistributionManifest::bundled());
            let assignment = script
                .lines()
                .find(|line| line.starts_with("$baseUrl ="))
                .unwrap();
            let output = Command::new("pwsh")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!("{assignment}; [Console]::Write($baseUrl)"),
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(String::from_utf8(output.stdout).unwrap(), origin);
        }
    }
}

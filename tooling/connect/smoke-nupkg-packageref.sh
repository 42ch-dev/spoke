#!/usr/bin/env bash
# Smoke 42ch.Spoke.Connect via PackageReference against a local nupkg feed.
# Exercises NuGet RID native resolution (not ProjectReference copy fallback).
#
# Usage (from repo root):
#   ./tooling/connect/smoke-nupkg-packageref.sh <nupkg-dir> [version]
#
# Defaults version from package.json when omitted.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NUPKG_DIR="$(cd "${1:?usage: $0 <nupkg-dir> [version]}" && pwd)"
VERSION="${2:-$(node -p "require('${REPO_ROOT}/package.json').version")}"

uname_s="$(uname -s)"
case "${uname_s}" in
  Linux) RID=linux-x64 ;;
  Darwin)
    if [[ "$(uname -m)" == "arm64" ]]; then RID=osx-arm64; else RID=osx-x64; fi
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) RID=win-x64 ;;
  *)
    echo "unsupported host: ${uname_s}" >&2
    exit 1
    ;;
esac

CONSUMER="$(mktemp -d)"
trap 'rm -rf "${CONSUMER}"' EXIT

cat >"${CONSUMER}/nuget.config" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="local-spoke-connect" value="${NUPKG_DIR}" />
  </packageSources>
</configuration>
EOF

cat >"${CONSUMER}/Smoke.csproj" <<EOF
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <RuntimeIdentifier>${RID}</RuntimeIdentifier>
    <AssemblyName>spoke-connect-packageref-smoke</AssemblyName>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="42ch.Spoke.Connect" Version="${VERSION}" />
    <Compile Include="${REPO_ROOT}/crates/spoke-connect/bindings/csharp/Smoke/Program.cs" />
    <Compile Include="${REPO_ROOT}/crates/spoke-connect/bindings/csharp/Smoke/tests/GoldenParity.cs" />
    <None Include="${REPO_ROOT}/crates/spoke-connect/bindings/csharp/Smoke/fixtures/golden-hello.json" CopyToOutputDirectory="PreserveNewest" Link="fixtures\golden-hello.json" />
  </ItemGroup>
</Project>
EOF

echo "PackageReference smoke: 42ch.Spoke.Connect ${VERSION} rid=${RID} feed=${NUPKG_DIR}"
dotnet restore "${CONSUMER}/Smoke.csproj" --configfile "${CONSUMER}/nuget.config"
dotnet run --project "${CONSUMER}/Smoke.csproj" -c Release --no-restore

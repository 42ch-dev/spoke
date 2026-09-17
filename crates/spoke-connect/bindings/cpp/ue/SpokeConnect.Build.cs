// SpokeConnect — Unreal Engine module template for the spoke-connect C ABI carrier.
//
// Reference module: it wires include paths and the committed native carriers of
// `crates/spoke-connect/bindings/cpp/` into an Unreal Engine target. The module
// directory is `cpp/ue/`, a sibling of the `include/` header tree and the
// `native/<rid>/` carrier tree it consumes, so the paths below resolve from
// `ModuleDirectory/..`; keep those three directories together when vendoring
// this reference into a project.
//
// The carrier is a dynamic library with a hand-written C ABI
// (`include/spoke_connect.h`, ABI revision 1). It links the Rust dynamic CRT,
// so the consuming Windows target uses the release C++ CRT (`/MD`). Carrier
// calls block the calling OS thread and execute their callbacks on the
// carrier's blocking pool; hosts call the ABI from worker threads and hand
// results to engine-owned queues. The carrier stays loaded for the process
// lifetime.
//
// Validation status: standalone macOS/Windows evidence is recorded separately;
// UE editor and packaged-game integration are unverified. See README.md in this
// directory for the maintainer's verification checklist.

using System.IO;
using UnrealBuildTool;

public class SpokeConnect : ModuleRules
{
	public SpokeConnect(ReadOnlyTargetRules Target) : base(Target)
	{
		Type = ModuleType.External;

		string BindingRoot = Path.GetFullPath(Path.Combine(ModuleDirectory, ".."));
		string IncludeDir = Path.Combine(BindingRoot, "include");
		string NativeDir = Path.Combine(BindingRoot, "native");

		// The C/C++ contract: include/spoke_connect.h
		PublicIncludePaths.Add(IncludeDir);

		if (Target.Platform == UnrealTargetPlatform.Win64)
		{
			if (Target.Architecture != UnrealArch.X64)
			{
				throw new BuildException(
					$"SpokeConnect: the committed Windows carrier is native/win-x64; architecture {Target.Architecture} is unsupported.");
			}

			string WinDir = Path.Combine(NativeDir, "win-x64");

			// Link the Rust-produced import library and stage the DLL next to the
			// executable, which is where the loader resolves it from.
			PublicAdditionalLibraries.Add(Path.Combine(WinDir, "spoke_connect_capi.dll.lib"));
			RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", Path.Combine(WinDir, "spoke_connect_capi.dll"));
		}
		else if (Target.Platform == UnrealTargetPlatform.Mac)
		{
			if (Target.Architecture != UnrealArch.Arm64)
			{
				throw new BuildException(
					$"SpokeConnect: the committed macOS carrier is native/osx-arm64; architecture {Target.Architecture} is unsupported.");
			}

			string MacDir = Path.Combine(NativeDir, "osx-arm64");

			// The carrier's install name is @rpath/libspoke_connect_capi.dylib, and
			// UnrealBuildTool adds the rpath for third-party dylibs outside Source.
			// The dylib stages as a loose runtime dependency.
			PublicAdditionalLibraries.Add(Path.Combine(MacDir, "libspoke_connect_capi.dylib"));
			RuntimeDependencies.Add("$(TargetOutputDir)/libspoke_connect_capi.dylib", Path.Combine(MacDir, "libspoke_connect_capi.dylib"), StagedFileType.NonUFS);
		}
		else
		{
			throw new BuildException(
				$"SpokeConnect: the committed carriers are native/osx-arm64 and native/win-x64; platform {Target.Platform} is unsupported.");
		}
	}
}

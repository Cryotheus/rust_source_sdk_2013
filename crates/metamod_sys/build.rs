use crys_bricks::config::{cargo_var, cargo_var_os, require_dir, require_file, require_var_os};
use std::path::PathBuf;

const ENV_MMS_STABLE: &str = "METAMOD_SOURCE_STABLE";
const ENV_MMS_DEV: &str = "METAMOD_SOURCE_DEV";

fn main() -> anyhow::Result<()> {
	let arch = cargo_var("CARGO_CFG_TARGET_ARCH")?.into_inner();
	let os = cargo_var("CARGO_CFG_TARGET_OS")?.into_inner();
	let env = cargo_var("CARGO_CFG_TARGET_ENV")?.into_inner();

	anyhow::ensure!(
		arch == "x86_64" && ((os == "windows" && env == "msvc") || (os == "linux" && env == "gnu")),
		"unsupported Metamod target: {arch}-{os}-{env}"
	);

	let manifest_dir = PathBuf::from(cargo_var_os("CARGO_MANIFEST_DIR")?.into_inner());
	let env_file = manifest_dir.join("../../.env");

	println!("cargo:rerun-if-changed={}", env_file.display());

	if env_file.is_file() {
		dotenv::from_path(&env_file)?;
	}

	// Keep the supplied path spelling: Windows MSVC does not reliably accept
	// the verbatim prefix added by Path::canonicalize.
	let stable_src = PathBuf::from(
		require_var_os(
			ENV_MMS_STABLE,
			"set it to the Metamod:Source 1.12 build 1226 source directory",
		)?
		.emit(),
	);

	let dev_src = PathBuf::from(
		require_var_os(
			ENV_MMS_DEV,
			"set it to the Metamod:Source 2.0 git1469 source directory with recursive submodules",
		)?
		.emit(),
	);

	require_dir(&stable_src, "Stable Metamod:Source directory")?;
	require_dir(&dev_src, "Dev Metamod:Source directory")?;

	require_file(
		&manifest_dir.join("src/cpp/bridge.cpp"),
		"Metamod C++ bridge",
	)?
	.emit();

	let stable_bridge = manifest_dir.join("src/cpp/stable.cpp");
	let stable_core = stable_src.join("core");
	let stable_sourcehook = stable_core.join("sourcehook");

	require_file(&stable_bridge, "Stable Metamod C++ bridge")?.emit();
	require_file(
		&stable_core.join("ISmmPlugin.h"),
		"Stable Metamod plugin API header",
	)?
	.emit();
	require_file(
		&stable_sourcehook.join("sourcehook.h"),
		"Stable SourceHook API header",
	)?
	.emit();

	let dev_bridge = manifest_dir.join("src/cpp/dev.cpp");
	let dev_core = dev_src.join("core");
	let dev_khook = dev_src.join("third_party/khook/include");

	require_file(&dev_bridge, "Dev Metamod C++ bridge")?.emit();
	require_file(
		&dev_core.join("ISmmPlugin.h"),
		"Dev Metamod plugin API header",
	)?
	.emit();
	require_file(&dev_khook.join("khook.hpp"), "Dev KHook API header")?.emit();

	compile_bridge(
		&stable_bridge,
		&stable_core,
		&stable_sourcehook,
		"metamod_source_bridge_stable",
		&env,
	);
	compile_bridge(
		&dev_bridge,
		&dev_core,
		&dev_khook,
		"metamod_source_bridge_dev",
		&env,
	);

	Ok(())
}

fn compile_bridge(
	bridge: &std::path::Path,
	core: &std::path::Path,
	dependency: &std::path::Path,
	archive: &str,
	target_env: &str,
) {
	let mut compiler = cc::Build::new();
	compiler
		.cpp(true)
		.file(bridge)
		.include(core)
		.include(dependency)
		.define("META_NO_HL2SDK", None)
		.std("c++17");

	if target_env == "msvc" {
		compiler.flag("/wd4100");
	} else {
		compiler.flag_if_supported("-Wno-unused-parameter");

		// SourceHook's FastDelegate casts between member function pointer types,
		// and its hook macros define helpers a plugin may not use.
		compiler.flag_if_supported("-Wno-cast-function-type");
		compiler.flag_if_supported("-Wno-unused-function");
	}

	compiler.compile(archive);
}

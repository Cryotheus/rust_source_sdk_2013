fn main() -> anyhow::Result<()> {
	println!("cargo:rerun-if-env-changed=SOURCE_SDK_2013");

	#[cfg(feature = "generate_bindings")]
	generate_bindings::generate_bindings()?;

	Ok(())
}

#[cfg(feature = "generate_bindings")]
mod generate_bindings {
	use source_sdk_2013_bindgen::SupportedTarget;
	use std::env::var_os;
	use std::path::PathBuf;

	pub(super) fn generate_bindings() -> anyhow::Result<()> {
		let manifest_dir = PathBuf::from(var_os("CARGO_MANIFEST_DIR").unwrap());
		let supported_target = SupportedTarget::from_env()?;
		let builder = source_sdk_2013_bindgen::Builder::new().build()?;
		let bindings = builder.generate_for(supported_target)?;
		let bindings_dir = manifest_dir.join("src").join(supported_target.short_name());

		// The SDK is supplied outside this Cargo workspace, so Cargo cannot infer
		// which headers affect the generated modules from package dependencies.
		// Ignore the synthetic in-memory bridge name and track every real header
		// observed by bindgen's include callbacks.
		for included_file in bindings
			.included_files()
			.iter()
			.filter(|path| path.is_file())
		{
			println!("cargo:rerun-if-changed={}", included_file.display());
		}

		eprintln!("Writing bindings to {bindings_dir:?}");
		bindings.write(bindings_dir)?;

		Ok(())
	}
}

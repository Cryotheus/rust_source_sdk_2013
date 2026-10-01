use clap::{Arg, value_parser};
use log::{LevelFilter, info};
use source_sdk_2013_bindgen::SupportedTarget;
use std::env::args;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
	let command = clap::Command::new("Rust Source SDK 2013 Bindgen Runner")
		.version(env!("CARGO_PKG_VERSION"))
		.arg(
			Arg::new("manifest-dir")
				.long("manifest-dir")
				.env("CARGO_MANIFEST_DIR")
				.required(true)
				.value_parser(value_parser!(PathBuf)),
		)
		.arg(
			Arg::new("target")
				.long("target")
				.short('t')
				.value_parser(value_parser!(SupportedTarget)),
		)
		.arg(
			Arg::new("dotenv")
				.long("dotenv")
				.value_parser(value_parser!(PathBuf)),
		);

	let args = command.get_matches();

	match args.get_one::<PathBuf>("dotenv") {
		None => {
			dotenv::dotenv().ok();
		}

		Some(path) => {
			dotenv::from_path(path)?;
		}
	};

	// UNWRAP: No other logger is set
	fern::Dispatch::new()
		.format(|out, args, record| out.finish(format_args!("[{}] {args}", record.level())))
		.chain(std::io::stderr())
		.level(LevelFilter::Off)
		.level_for("source_sdk_2013_bindgen", LevelFilter::Trace)
		.apply()
		.unwrap();

	let supported_target = args
		.get_one::<SupportedTarget>("target")
		.cloned()
		.ok_or(())
		.or(SupportedTarget::from_env())
		.or(SupportedTarget::from_host())?;

	let manifest_dir = args.get_one::<PathBuf>("manifest-dir").unwrap();
	let manifest_dir = manifest_dir.canonicalize()?;

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

	info!("Writing bindings to {bindings_dir:?}");
	bindings.write(bindings_dir)?;
	Ok(())
}

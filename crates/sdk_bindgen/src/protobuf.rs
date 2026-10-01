//! Prepare Valve's generated protocol headers without modifying the SDK checkout.
//!
//! The SDK ships protobuf 2.6.1 and its matching compiler, but not the game's
//! generated headers. Newer system protoc releases are not ABI/header compatible
//! with that runtime. Output belongs to the consuming build's OUT_DIR, or a
//! temporary directory owned by standalone generation calls.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const PROTOS: &[&str] = &[
	"gcsdk/steammessages.proto",
	"game/shared/base_gcmessages.proto",
	"game/shared/econ/econ_gcmessages.proto",
	"game/shared/tf/tf_gcmessages.proto",
	"game/shared/tf/tf_proto_def_messages.proto",
];

#[derive(Debug)]
pub(crate) struct Prepared {
	pub include_dir: PathBuf,
	/// Original inputs must be tracked as well as included generated headers.
	pub inputs: Vec<PathBuf>,
	/// Standalone API calls keep their temporary headers alive through generation.
	_temporary_root: Option<Staging>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProtobufError {
	#[error("protobuf preparation failed for {path:?}: {source}")]
	Io {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	#[error("the SDK's bundled protobuf compiler is unavailable for this host OS")]
	UnsupportedHost,
	#[error("SDK protobuf compiler {compiler:?} failed ({status}): {diagnostic}")]
	Compiler {
		compiler: PathBuf,
		status: String,
		diagnostic: String,
	},
}

fn io_error(path: &Path, source: std::io::Error) -> ProtobufError {
	ProtobufError::Io {
		path: path.to_owned(),
		source,
	}
}

fn compiler_path(sdk_src: &Path) -> Result<PathBuf, ProtobufError> {
	let relative = if cfg!(target_os = "windows") {
		"thirdparty/protobuf-2.6.1/bin/win64/2015/staticcrt/release/protoc.exe"
	} else if cfg!(target_os = "linux") {
		// Valve's VPC uses this compiler for both Linux target ABIs. The
		// executable's host architecture does not affect generated C++ headers.
		"thirdparty/protobuf-2.6.1/bin/linux32/protoc"
	} else {
		return Err(ProtobufError::UnsupportedHost);
	};
	Ok(sdk_src.join(relative))
}

/// Cargo keeps generated headers in OUT_DIR; standalone callers need no Cargo
/// environment and release their private output when generation finishes.
pub(crate) fn prepare_for_build(sdk_src: &Path) -> Result<Prepared, ProtobufError> {
	let out_dir = std::env::var_os("OUT_DIR");
	prepare_with_output(sdk_src, out_dir.as_deref().map(Path::new))
}

fn prepare_with_output(sdk_src: &Path, out_dir: Option<&Path>) -> Result<Prepared, ProtobufError> {
	if let Some(out_dir) = out_dir {
		return prepare(sdk_src, out_dir);
	}
	let temporary_root = Staging::new(&std::env::temp_dir())?;
	let mut prepared = prepare(sdk_src, &temporary_root.0)?;
	prepared._temporary_root = Some(temporary_root);
	Ok(prepared)
}

pub(crate) fn prepare(sdk_src: &Path, out_dir: &Path) -> Result<Prepared, ProtobufError> {
	let compiler = compiler_path(sdk_src)?;
	let mut inputs: Vec<_> = PROTOS.iter().map(|path| sdk_src.join(path)).collect();
	inputs.push(sdk_src.join("thirdparty/protobuf-2.6.1/src/google/protobuf/descriptor.proto"));
	inputs.push(compiler.clone());
	for input in &inputs {
		fs::metadata(input).map_err(|error| io_error(input, error))?;
	}

	let include_dir = out_dir.join("source-sdk-protobuf");
	fs::create_dir_all(&include_dir).map_err(|error| io_error(&include_dir, error))?;
	let staging = Staging::new(out_dir)?;
	let mut command = Command::new(&compiler);
	for relative in [
		"thirdparty/protobuf-2.6.1/src",
		// Narrow game directories must precede game/shared so protoc emits
		// tf_gcmessages.pb.h, not tf/tf_gcmessages.pb.h. SDK includes are flat.
		"game/shared/tf",
		"game/shared/econ",
		"gcsdk",
		"game/shared",
	] {
		command.arg(format!("--proto_path={}", sdk_src.join(relative).display()));
	}
	command.arg(format!("--cpp_out={}", staging.0.display()));
	command.args(PROTOS.iter().map(|path| sdk_src.join(path)));
	let output = command
		.output()
		.map_err(|error| io_error(&compiler, error))?;
	if !output.status.success() {
		return Err(ProtobufError::Compiler {
			compiler,
			status: output.status.to_string(),
			diagnostic: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
		});
	}
	for proto in PROTOS {
		let name = Path::new(proto)
			.file_stem()
			.expect("fixed proto has a filename");
		let mut header = name.to_os_string();
		header.push(".pb.h");
		let source = staging.0.join(&header);
		let destination = include_dir.join(&header);
		let contents = fs::read(&source).map_err(|error| io_error(&source, error))?;
		write_changed(&destination, &contents)?;
	}
	Ok(Prepared {
		include_dir,
		inputs,
		_temporary_root: None,
	})
}

/// Preserve timestamps for generated headers tracked by Cargo's include callbacks.
fn write_changed(path: &Path, contents: &[u8]) -> Result<(), ProtobufError> {
	match fs::read(path) {
		Ok(previous) if previous == contents => return Ok(()),
		Ok(_) => (),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
		Err(error) => return Err(io_error(path, error)),
	}
	fs::write(path, contents).map_err(|error| io_error(path, error))
}

#[derive(Debug)]
struct Staging(PathBuf);

impl Staging {
	fn new(out_dir: &Path) -> Result<Self, ProtobufError> {
		static NEXT: AtomicUsize = AtomicUsize::new(0);
		loop {
			let path = out_dir.join(format!(
				".source-sdk-protoc-{}-{}",
				std::process::id(),
				NEXT.fetch_add(1, Ordering::Relaxed)
			));
			match fs::create_dir(&path) {
				Ok(()) => return Ok(Self(path)),
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(error) => return Err(io_error(&path, error)),
			}
		}
	}
}

impl Drop for Staging {
	fn drop(&mut self) {
		// This unique directory was exclusively created by Staging::new. It
		// contains protoc's disposable .pb.cc/.pb.h output, never source files.
		let _ = fs::remove_dir_all(&self.0);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn generated_headers_keep_timestamps_when_unchanged() {
		let directory = Staging::new(&std::env::temp_dir()).unwrap();
		let header = directory.0.join("sample.pb.h");
		write_changed(&header, b"first").unwrap();
		let original = fs::metadata(&header).unwrap().modified().unwrap();
		write_changed(&header, b"first").unwrap();
		assert_eq!(fs::metadata(&header).unwrap().modified().unwrap(), original);
		write_changed(&header, b"second").unwrap();
		assert_eq!(fs::read(header).unwrap(), b"second");
	}

	#[test]
	#[ignore = "set SOURCE_SDK_2013 to an authorized SDK checkout with bundled protoc"]
	fn bundled_compiler_generates_all_tf2_dependencies_outside_the_sdk() {
		let sdk = PathBuf::from(std::env::var_os("SOURCE_SDK_2013").expect("SOURCE_SDK_2013"));
		let output = Staging::new(&std::env::temp_dir()).unwrap();
		let prepared = prepare(&sdk.join("src"), &output.0).unwrap();
		assert_eq!(prepared.inputs.len(), PROTOS.len() + 2);
		for proto in PROTOS {
			let header = prepared.include_dir.join(format!(
				"{}.pb.h",
				Path::new(proto).file_stem().unwrap().to_str().unwrap()
			));
			let text = fs::read_to_string(header).unwrap();
			assert!(text.contains("Generated by the protocol buffer compiler"));
		}
		assert_eq!(
			fs::read_dir(&output.0).unwrap().count(),
			1,
			"disposable compiler outputs must be removed"
		);
	}

	#[test]
	#[ignore = "set SOURCE_SDK_2013 to an authorized SDK checkout with bundled protoc"]
	fn standalone_generation_owns_and_removes_its_temporary_headers() {
		let sdk = PathBuf::from(std::env::var_os("SOURCE_SDK_2013").expect("SOURCE_SDK_2013"));
		let prepared = prepare_with_output(&sdk.join("src"), None).unwrap();
		let root = prepared._temporary_root.as_ref().unwrap().0.clone();
		assert!(prepared.include_dir.join("tf_gcmessages.pb.h").is_file());
		drop(prepared);
		assert!(!root.exists());
	}
}

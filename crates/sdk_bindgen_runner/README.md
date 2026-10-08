# Bindgen Runner for `rust_source_sdk_2013`

Solely for running [`source_sdk_2013_bindgen`](../sdk_bindgen) as a standalone executable.

**You shouldn't need to run this!**  
**Ready-to-use pre-generated bindings are part of [`source_sdk_2013_sys`](../sdk_sys) already!**

This is for development of `rust_source_sdk_2013` itself,
and is not required for usage of the repository's crates as dependencies in your own project.

## Why?

Previously, bindings were generated through a `generate-bindings` feature on [`source_sdk_2013_sys`](../sdk_sys),
but doing so presented a few problems:

- It became easy to accidentally re-generate bindings
	- Usage of the `--all-features` flag could trigger the build script to generate bindings
	- Some IDEs would run the build script which also re-generated the bindings
- The output of a build script (`build.rs`) is not displayed during `cargo build` / `cargo check`
	- Generation takes a while, a lack of feedback makes it seem like the build process is hanging
- *and more I forgot throughout the process of migrating away from using a build script*

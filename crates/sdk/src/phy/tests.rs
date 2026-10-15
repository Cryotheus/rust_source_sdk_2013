use super::*;

#[test]
#[ignore = "needs SOURCE_PHY_DIR pointing at extracted .phy files beside their .mdlhdr headers"]
fn real_phy_files_fit_their_model_bounds() {
	let root =
		std::path::PathBuf::from(std::env::var_os("SOURCE_PHY_DIR").expect("fixture directory"));
	let mut pending = vec![root];
	let (mut files, mut hulls, mut outside) = (0, 0, 0);
	while let Some(directory) = pending.pop() {
		for entry in std::fs::read_dir(directory).unwrap() {
			let path = entry.unwrap().path();
			if path.is_dir() {
				pending.push(path);
				continue;
			}
			if path.extension().is_none_or(|extension| extension != "phy") {
				continue;
			}
			let decoded = decode(&std::fs::read(&path).unwrap())
				.unwrap_or_else(|error| panic!("{}: {error}", path.display()));
			assert!(!decoded.is_empty(), "{}", path.display());
			// studiohdr_t keeps the movement hull bounds at bytes 104 to 128.
			let header = std::fs::read(path.with_extension("mdlhdr")).ok();
			let bounds = header.filter(|header| header.len() >= 128).map(|header| {
				let read = |at: usize| {
					Vec3::from_array(std::array::from_fn(|axis| {
						f32::from_le_bytes(header[at + axis * 4..][..4].try_into().unwrap())
					}))
				};
				(read(104), read(116))
			});
			for hull in &decoded {
				assert!(
					hull.triangles.len() >= 4,
					"{}: a convex piece needs four faces",
					path.display()
				);
				for triangle in &hull.triangles {
					assert!(
						triangle
							.iter()
							.all(|&index| (index as usize) < hull.points.len())
					);
				}
				if let Some((min, max)) = bounds {
					let slack = (max - min).max_element() * 0.1 + 4.0;
					if hull.points.iter().any(|point| {
						point.cmplt(min - slack).any() || point.cmpgt(max + slack).any()
					}) {
						outside += 1;
					}
				}
			}
			files += 1;
			hulls += decoded.len();
		}
	}
	println!("{files} files, {hulls} convex pieces, {outside} pieces outside their model hull");
	assert!(files > 0);
	assert!(
		outside * 20 <= hulls,
		"too many pieces fall outside their model bounds: axes or scale are wrong"
	);
}

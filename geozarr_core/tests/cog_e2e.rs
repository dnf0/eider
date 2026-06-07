use geozarr_core::dataset::ZarrDataset;
use geozarr_core::query_planner::QueryConstraints;
use std::collections::HashMap;

fn fixture(name: &str) -> String {
    // The local-path access gate in `resolve_sync_store` only permits paths under
    // `GEOZARR_ALLOW_PATH` (or CWD). The committed fixtures live under the crate
    // manifest dir, which is independent of the test runner's working directory,
    // so point the sandbox at the manifest dir. All tests set the same value, so
    // this is safe to set unconditionally even under parallel execution.
    std::env::set_var("GEOZARR_ALLOW_PATH", env!("CARGO_MANIFEST_DIR"));
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}

#[test]
fn cog_metadata_is_georeferenced() {
    let ds = ZarrDataset::open(&fixture("cog_int16_uncompressed.tif")).unwrap();
    assert_eq!(ds.dim_names, vec!["lat".to_string(), "lon".to_string()]);
    assert!(
        ds.spatial_transform.is_some(),
        "affine transform must be present"
    );
    let schema = ds.schema().unwrap();
    // value column dtype is Int16 (not Float32)
    let (vname, vtype) = schema.last().unwrap();
    assert_eq!(vname, "value");
    assert_eq!(
        format!("{vtype:?}"),
        format!("{:?}", zarrs::array::DataType::Int16)
    );
}

#[test]
fn cog_bbox_prunes_via_lat_lon() {
    let ds = ZarrDataset::open(&fixture("cog_int16_uncompressed.tif")).unwrap();
    // Full extent: lon in [-180,-174], lat in [86,90] (origin -180/90, 2deg, 4x2).
    // Constrain to the western half (lon <= -177) -> fewer columns.
    let mut bounds = HashMap::new();
    bounds.insert("lon".to_string(), (None, Some(-177.0)));
    let constraints = QueryConstraints {
        bounds,
        pins: HashMap::new(),
    };
    let (bmin, bmax) = ds.compute_bounds(&constraints);
    // lon dim is index 1; with scale +2 translation -180, lon=-177 -> col ~1.5 -> max col 1
    assert!(
        bmax[1] < (ds.shape[1] - 1),
        "bbox should prune the lon dimension: {bmin:?}..{bmax:?}"
    );
}

#[test]
fn cog_deflate_matches_uncompressed_metadata() {
    let a = ZarrDataset::open(&fixture("cog_int16_uncompressed.tif")).unwrap();
    let b = ZarrDataset::open(&fixture("cog_int16_deflate.tif")).unwrap();
    assert_eq!(a.shape, b.shape);
    assert_eq!(a.dim_names, b.dim_names);
}

#[test]
fn cog_deflate_matches_uncompressed_values() {
    // Read the full array through the public `zarrs::Array` exposed on
    // `ZarrDataset`, decoding both the deflate-compressed and uncompressed
    // COGs end-to-end, and assert they decode to identical i16 values.
    let a = ZarrDataset::open(&fixture("cog_int16_uncompressed.tif")).unwrap();
    let b = ZarrDataset::open(&fixture("cog_int16_deflate.tif")).unwrap();

    let subset = zarrs::array_subset::ArraySubset::new_with_shape(a.shape.clone());
    let a_vals: Vec<i16> = a
        .array
        .retrieve_array_subset_elements::<i16>(&subset)
        .unwrap();
    let b_vals: Vec<i16> = b
        .array
        .retrieve_array_subset_elements::<i16>(&subset)
        .unwrap();

    assert!(!a_vals.is_empty(), "array must have values");
    assert_eq!(
        a_vals, b_vals,
        "deflate and uncompressed COGs must decode element-for-element"
    );
}

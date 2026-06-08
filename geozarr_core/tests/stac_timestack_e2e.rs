use geozarr_core::dataset::ZarrDataset;
use geozarr_core::query_planner::QueryConstraints;
use std::collections::HashMap;
use zarrs::array_subset::ArraySubset;

fn fixt(name: &str) -> String {
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}
fn allow() {
    std::env::set_var("GEOZARR_ALLOW_PATH", env!("CARGO_MANIFEST_DIR"));
}

#[test]
fn timestack_opens_as_3d_with_time_coords() {
    allow();
    let ds = ZarrDataset::open_with_asset(&fixt("stac_itemcollection.json"), Some("band")).unwrap();
    assert_eq!(
        ds.dim_names,
        vec!["time".to_string(), "lat".to_string(), "lon".to_string()]
    );
    assert_eq!(ds.shape, vec![2u64, 2, 4]);
    let time = ds.coords.get("time").expect("time coords present");
    // DIAGNOSTIC (temporary): dump the resolved time coords + first cell of each
    // slice to capture Windows-vs-mac ordering on CI.
    {
        use zarrs::array_subset::ArraySubset;
        let vals: Vec<i16> = ds
            .array
            .retrieve_array_subset_elements::<i16>(&ArraySubset::new_with_shape(vec![2, 2, 4]))
            .unwrap();
        eprintln!(
            "DIAG time={time:?} slice0[0,0]={} slice1[0,0]={}",
            vals[0], vals[8]
        );
        eprintln!("DIAG datetimes parsed: jan_epoch should be 1767225600");
    }
    // 2026-01-01 and 2026-02-01 in epoch seconds, ascending
    assert_eq!(time.len(), 2);
    assert!(time[0] < time[1]);
    assert_eq!(time[0], 1767225600.0);
    // value dtype is Int16
    let (vname, vtype) = ds.schema().unwrap().pop().unwrap();
    assert_eq!(vname, "value");
    assert_eq!(
        format!("{vtype:?}"),
        format!("{:?}", zarrs::array::DataType::Int16)
    );
}

#[test]
fn timestack_slices_read_distinct_per_item_data() {
    // Each stack slice must route to its OWN item's COG, not always item 0.
    // item0 -> cog_int16_uncompressed.tif (values row*10+col, so cell [0,0] == 0)
    // item1 -> cog_int16_alt.tif          (values base + 100, so cell [0,0] == 100)
    allow();
    let ds = ZarrDataset::open_with_asset(&fixt("stac_itemcollection.json"), Some("band")).unwrap();

    // Read the full 3D [time, lat, lon] = [2, 2, 4] array.
    let vals: Vec<i16> = ds
        .array
        .retrieve_array_subset_elements::<i16>(&ArraySubset::new_with_shape(vec![2, 2, 4]))
        .unwrap();
    assert_eq!(vals.len(), 16, "expected 2*2*4 elements");

    // Row-major: slice t occupies elements [t*8 .. t*8+8); cell [0,0] is the
    // first element of each slice.
    let slice0_00 = vals[0];
    let slice1_00 = vals[8];

    assert_eq!(
        slice0_00, 0,
        "slice 0 cell [0,0] should be item0's base value 0"
    );
    assert_eq!(
        slice1_00, 100,
        "slice 1 cell [0,0] should be item1 (alt fixture) value 100"
    );
    // The whole point: the two slices carry distinct data, proving slice t reads
    // item t. A "always read item 0" routing bug would make these equal.
    assert_ne!(
        slice0_00, slice1_00,
        "slices must differ; identical values would mask an 'always item 0' routing bug"
    );
}

#[test]
fn timestack_time_pushdown_prunes_to_one_slice() {
    allow();
    let ds = ZarrDataset::open_with_asset(&fixt("stac_itemcollection.json"), Some("band")).unwrap();
    // bracket only the first datetime
    let mut bounds = HashMap::new();
    bounds.insert(
        "time".to_string(),
        (Some(1767225600.0 - 10.0), Some(1767225600.0 + 10.0)),
    );
    let constraints = QueryConstraints {
        bounds,
        pins: HashMap::new(),
    };
    let (bmin, bmax) = ds.compute_bounds(&constraints);
    assert_eq!(
        (bmin[0], bmax[0]),
        (0, 0),
        "time should prune to index 0 only"
    );
}

// geozarr_core/src/virtual_store.rs
use crate::cog::CogMetadata;
use bytes::Bytes;
use zarrs::byte_range::ByteRange;
use zarrs::storage::{ListableStorageTraits, ReadableStorageTraits, StoreKey, StorePrefix};

pub struct VirtualCogStore {
    operator: opendal::Operator,
    filename: String,
    meta: CogMetadata,
    zarray_bytes: Bytes,
    zattrs_bytes: Bytes,
    zmetadata_bytes: Bytes,
}

impl VirtualCogStore {
    pub fn new(
        operator: opendal::Operator,
        filename: String,
        meta: CogMetadata,
    ) -> Result<Self, String> {
        // Synthesize honest Zarr-V2 metadata from the parsed COG.
        // `zarr_dtype()` is the guard that rejects multi-band COGs and
        // unsupported bit-depths/sample-formats; propagate its error so an
        // unsupported COG fails loudly at open time rather than silently
        // decoding as <f4 garbage.
        let dtype = meta.zarr_dtype()?;
        // zarrs' Zarr-V2 reader rejects a null fill value for integer data types,
        // so when the COG carries no GDAL_NODATA tag we fall back to a concrete 0
        // sentinel (valid for every supported dtype) rather than `null`.
        let fill = match meta.nodata {
            Some(v) => format!("{v}"),
            None => "0".to_string(),
        };
        let dims = meta.dim_names(); // ["lat","lon"] or ["y","x"]
        let dims_json = format!("[\"{}\", \"{}\"]", dims[0], dims[1]);
        let geozarr = match (meta.spatial_transform(), meta.crs()) {
            (Some(t), crs) => {
                let crs_json = crs
                    .map(|c| format!("\"crs\": \"{c}\","))
                    .unwrap_or_default();
                format!(
                    "{{ {} \"spatial_transform\": {{ \"scale\": [{}, {}], \"translation\": [{}, {}] }} }}",
                    crs_json, t.scale[0], t.scale[1], t.translation[0], t.translation[1]
                )
            }
            (None, _) => "{}".to_string(),
        };
        let zarray = format!(
            r#"{{"zarr_format":2,"shape":[{},{}],"chunks":[{},{}],"dtype":"{}","compressor":null,"fill_value":{},"filters":null,"order":"C"}}"#,
            meta.image_length, meta.image_width, meta.tile_length, meta.tile_width, dtype, fill
        );
        let zattrs = format!(
            r#"{{"_ARRAY_DIMENSIONS":{},"geozarr":{}}}"#,
            dims_json, geozarr
        );
        let zmetadata = format!(
            r#"{{"metadata":{{".zarray":{},".zattrs":{}}},"zarr_consolidated_format":1}}"#,
            zarray, zattrs
        );

        Ok(Self {
            operator,
            filename,
            meta,
            zarray_bytes: Bytes::from(zarray),
            zattrs_bytes: Bytes::from(zattrs),
            zmetadata_bytes: Bytes::from(zmetadata),
        })
    }
}

impl ReadableStorageTraits for VirtualCogStore {
    fn get(&self, key: &StoreKey) -> Result<Option<Bytes>, zarrs::storage::StorageError> {
        if key.as_str() == ".zmetadata" {
            return Ok(Some(self.zmetadata_bytes.clone()));
        }
        if key.as_str() == ".zarray" {
            return Ok(Some(self.zarray_bytes.clone()));
        }
        if key.as_str() == ".zattrs" {
            return Ok(Some(self.zattrs_bytes.clone()));
        }

        let chunks: Vec<&str> = key.as_str().split('.').collect();
        if chunks.len() == 2 {
            if let (Ok(y), Ok(x)) = (chunks[0].parse::<usize>(), chunks[1].parse::<usize>()) {
                let grid_width =
                    (self.meta.image_width as f64 / self.meta.tile_width as f64).ceil() as usize;
                let flat_idx = y * grid_width + x;

                if flat_idx < self.meta.tile_offsets.len() {
                    let offset = self.meta.tile_offsets[flat_idx];
                    let length = self.meta.tile_byte_counts[flat_idx];

                    let op = self.operator.clone();
                    let fname = self.filename.clone();
                    let range = offset..offset + length;
                    // Spawning a new thread to block on the async read
                    let bytes_res = std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async { op.read_with(&fname).range(range).await })
                            .map_err(|e| e.to_string())
                    })
                    .join()
                    .unwrap();

                    if let Ok(bytes) = bytes_res {
                        let raw = bytes.to_vec();
                        let decoded = match self.meta.compression_kind() {
                            Ok(crate::cog::CogCompression::None) => raw,
                            Ok(crate::cog::CogCompression::Deflate) => {
                                use std::io::Read;
                                let mut d = flate2::read::ZlibDecoder::new(&raw[..]);
                                let mut out = Vec::new();
                                d.read_to_end(&mut out).map_err(|e| {
                                    zarrs::storage::StorageError::Other(format!(
                                        "deflate decode failed: {e}"
                                    ))
                                })?;
                                out
                            }
                            Err(e) => return Err(zarrs::storage::StorageError::Other(e)),
                        };
                        return Ok(Some(Bytes::from(decoded)));
                    }
                }
            }
        }
        Ok(None)
    }

    fn get_partial_values_key(
        &self,
        key: &StoreKey,
        byte_ranges: &[ByteRange],
    ) -> Result<Option<Vec<Bytes>>, zarrs::storage::StorageError> {
        // Simplified fallback to full chunk fetch for minimal impl
        if let Some(bytes) = self.get(key)? {
            let mut out = Vec::new();
            for r in byte_ranges {
                let start = match r {
                    ByteRange::FromStart(offset, _) => *offset,
                    _ => 0,
                };
                let end = match r {
                    ByteRange::FromStart(offset, Some(len)) => *offset + *len,
                    _ => bytes.len() as u64,
                };
                let slice = bytes.slice(start as usize..end as usize);
                out.push(slice);
            }
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }

    fn size_key(&self, _key: &StoreKey) -> Result<Option<u64>, zarrs::storage::StorageError> {
        Ok(None)
    }
}

impl ListableStorageTraits for VirtualCogStore {
    fn list(&self) -> Result<zarrs::storage::StoreKeys, zarrs::storage::StorageError> {
        Ok(vec![
            StoreKey::new(".zmetadata").unwrap(),
            StoreKey::new(".zarray").unwrap(),
            StoreKey::new(".zattrs").unwrap(),
        ])
    }
    fn list_prefix(
        &self,
        _prefix: &StorePrefix,
    ) -> Result<zarrs::storage::StoreKeys, zarrs::storage::StorageError> {
        Ok(vec![])
    }
    fn list_dir(
        &self,
        _prefix: &StorePrefix,
    ) -> Result<zarrs::storage::StoreKeysPrefixes, zarrs::storage::StorageError> {
        unimplemented!("Not needed for minimal test")
    }
    fn size_prefix(&self, _prefix: &StorePrefix) -> Result<u64, zarrs::storage::StorageError> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_virtual_store_synthesizes_geozarr_attrs() {
        use zarrs::storage::ReadableStorageTraits;
        let mut meta = crate::cog::CogMetadata {
            image_width: 4,
            image_length: 2,
            tile_width: 4,
            tile_length: 2,
            tile_offsets: vec![0],
            tile_byte_counts: vec![16],
            is_little_endian: true,
            bits_per_sample: 16,
            sample_format: 2,
            samples_per_pixel: 1,
            compression: 1,
            predictor: 1,
            ..Default::default()
        };
        meta.pixel_scale = vec![2.0, 2.0, 0.0];
        meta.tiepoint = vec![0.0, 0.0, 0.0, -180.0, 90.0, 0.0];
        meta.epsg = Some(4326);

        let op = opendal::Operator::new(opendal::services::Memory::default())
            .unwrap()
            .finish();
        let store = VirtualCogStore::new(op, "".to_string(), meta).unwrap();

        let zarray = String::from_utf8(
            store
                .get(&zarrs::storage::StoreKey::new(".zarray").unwrap())
                .unwrap()
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(zarray.contains("\"<i2\""), "dtype should be <i2: {zarray}");

        let zattrs = String::from_utf8(
            store
                .get(&zarrs::storage::StoreKey::new(".zattrs").unwrap())
                .unwrap()
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(zattrs.contains("_ARRAY_DIMENSIONS"));
        assert!(zattrs.contains("\"lat\"") && zattrs.contains("\"lon\""));
        assert!(zattrs.contains("EPSG:4326"));
        assert!(zattrs.contains("spatial_transform"));
    }

    #[tokio::test]
    async fn test_multiband_cog_rejected_at_open() {
        // A multi-band COG (samples_per_pixel != 1) is otherwise valid but
        // unsupported; opening it must fail loudly rather than silently
        // misreading the data as single-band <f4.
        let meta = crate::cog::CogMetadata {
            image_width: 4,
            image_length: 2,
            tile_width: 4,
            tile_length: 2,
            tile_offsets: vec![0],
            tile_byte_counts: vec![16],
            is_little_endian: true,
            bits_per_sample: 16,
            sample_format: 2,
            samples_per_pixel: 3,
            compression: 1,
            predictor: 1,
            ..Default::default()
        };
        let op = opendal::Operator::new(opendal::services::Memory::default())
            .unwrap()
            .finish();
        assert!(VirtualCogStore::new(op, "".to_string(), meta).is_err());
    }

    #[tokio::test]
    async fn test_deflate_tile_is_inflated() {
        use std::io::Write;
        use zarrs::storage::ReadableStorageTraits;
        // raw 2x4 i16 LE tile = 16 bytes
        let raw: Vec<u8> = (0..16u8).collect();
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();

        let op = opendal::Operator::new(opendal::services::Memory::default())
            .unwrap()
            .finish();
        op.write("tile.bin", compressed.clone()).await.unwrap();

        let meta = crate::cog::CogMetadata {
            image_width: 4,
            image_length: 2,
            tile_width: 4,
            tile_length: 2,
            tile_offsets: vec![0],
            tile_byte_counts: vec![compressed.len() as u64],
            is_little_endian: true,
            bits_per_sample: 16,
            sample_format: 2,
            samples_per_pixel: 1,
            compression: 8,
            predictor: 1,
            ..Default::default()
        };
        let store = VirtualCogStore::new(op, "tile.bin".to_string(), meta).unwrap();
        let out = store
            .get(&zarrs::storage::StoreKey::new("0.0").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            out.to_vec(),
            raw,
            "deflate tile must be inflated to raw bytes"
        );
    }
}

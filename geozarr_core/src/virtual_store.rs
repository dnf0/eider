// geozarr_core/src/virtual_store.rs
use crate::cog::CogMetadata;
use bytes::Bytes;
use zarrs::byte_range::ByteRange;
use zarrs::storage::{ListableStorageTraits, ReadableStorageTraits, StoreKey, StorePrefix};

pub struct VirtualCogStore {
    operator: opendal::Operator,
    filename: String,
    meta: CogMetadata,
    zmetadata_bytes: Bytes,
}

impl VirtualCogStore {
    pub fn new(operator: opendal::Operator, filename: String, meta: CogMetadata) -> Self {
        // Synthesize a .zmetadata JSON for a root array
        let json = format!(
            r#"{{
            "metadata": {{
                ".zarray": {{
                    "zarr_format": 2,
                    "shape": [{}, {}],
                    "chunks": [{}, {}],
                    "dtype": "<f4",
                    "compressor": null,
                    "fill_value": null,
                    "filters": null,
                    "order": "C"
                }}
            }},
            "zarr_consolidated_format": 1
        }}"#,
            meta.image_length, meta.image_width, meta.tile_length, meta.tile_width
        );

        Self {
            operator,
            filename,
            meta,
            zmetadata_bytes: Bytes::from(json),
        }
    }
}

impl ReadableStorageTraits for VirtualCogStore {
    fn get(&self, key: &StoreKey) -> Result<Option<Bytes>, zarrs::storage::StorageError> {
        if key.as_str() == ".zmetadata" {
            return Ok(Some(self.zmetadata_bytes.clone()));
        }
        if key.as_str() == ".zarray" {
            let json = format!(
                r#"{{
                "zarr_format": 2,
                "shape": [{}, {}],
                "chunks": [{}, {}],
                "dtype": "<f4",
                "compressor": null,
                "fill_value": "NaN",
                "filters": null,
                "order": "C"
            }}"#,
                self.meta.image_length, self.meta.image_width, self.meta.tile_length, self.meta.tile_width
            );
            return Ok(Some(Bytes::from(json)));
        }

        let chunks: Vec<&str> = key.as_str().split('.').collect();
        if chunks.len() == 2 {
            if let (Ok(y), Ok(x)) = (chunks[0].parse::<usize>(), chunks[1].parse::<usize>()) {

            let grid_width = (self.meta.image_width as f64 / self.meta.tile_width as f64)
                .ceil() as usize;
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
                            return Ok(Some(Bytes::from(bytes.to_vec())));
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
        Ok(vec![StoreKey::new(".zmetadata").unwrap()])
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
    async fn test_virtual_store_metadata() {
        let meta = crate::cog::CogMetadata {
            image_width: 1024,
            image_length: 1024,
            tile_width: 256,
            tile_length: 256,
            tile_offsets: vec![100, 200, 300, 400],
            tile_byte_counts: vec![50, 50, 50, 50],
        };

        let op = opendal::Operator::new(opendal::services::Memory::default())
            .unwrap()
            .finish();
        let store = VirtualCogStore::new(op, meta);

        // Zarrs ReadableStorageTraits implementation test
        use zarrs::storage::ReadableStorageTraits;
        let md = store
            .get(&zarrs::storage::StoreKey::new(".zmetadata").unwrap())
            .unwrap()
            .unwrap();
        let md_str = String::from_utf8(md.to_vec()).unwrap();
        assert!(md_str.contains("zarr_format"));
        assert!(md_str.contains("1024"));
    }
}

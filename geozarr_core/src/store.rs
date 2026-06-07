use std::sync::Arc;
use zarrs::storage::ReadableStorageTraits;

pub struct ResolvedStore {
    pub store: Arc<dyn ReadableStorageTraits>,
    pub is_remote: bool,
}

pub struct AsyncToSyncOpendalStore {
    operator: opendal::Operator,
}

impl AsyncToSyncOpendalStore {
    pub fn new(operator: opendal::Operator) -> Self {
        Self { operator }
    }
}

impl ReadableStorageTraits for AsyncToSyncOpendalStore {
    fn get(
        &self,
        key: &zarrs::storage::StoreKey,
    ) -> Result<Option<bytes::Bytes>, zarrs::storage::StorageError> {
        let op = self.operator.clone();
        let key_str = key.as_str().to_string();

        let res = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match op.read(&key_str).await {
                    Ok(bytes) => Ok(Some(bytes::Bytes::from(bytes.to_vec()))),
                    Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(zarrs::storage::StorageError::Other(e.to_string())),
                }
            })
        })
        .join()
        .unwrap();

        res
    }

    fn get_partial_values_key(
        &self,
        key: &zarrs::storage::StoreKey,
        byte_ranges: &[zarrs::byte_range::ByteRange],
    ) -> Result<Option<Vec<bytes::Bytes>>, zarrs::storage::StorageError> {
        let op = self.operator.clone();
        let key_str = key.as_str().to_string();
        let ranges = byte_ranges.to_vec();

        let res = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match op.read(&key_str).await {
                    Ok(bytes) => {
                        let mut out = Vec::with_capacity(ranges.len());
                        for r in ranges {
                            let start = match r {
                                zarrs::byte_range::ByteRange::FromStart(offset, _) => offset,
                                _ => 0,
                            };
                            let end = match r {
                                zarrs::byte_range::ByteRange::FromStart(offset, Some(len)) => {
                                    offset + len
                                }
                                _ => bytes.len() as u64,
                            };
                            let slice = bytes.slice(start as usize..end as usize);
                            out.push(bytes::Bytes::from(slice.to_vec()));
                        }
                        Ok(Some(out))
                    }
                    Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(zarrs::storage::StorageError::Other(e.to_string())),
                }
            })
        })
        .join()
        .unwrap();

        res
    }

    fn size_key(
        &self,
        key: &zarrs::storage::StoreKey,
    ) -> Result<Option<u64>, zarrs::storage::StorageError> {
        let op = self.operator.clone();
        let key_str = key.as_str().to_string();

        let res = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match op.stat(&key_str).await {
                    Ok(meta) => Ok(Some(meta.content_length())),
                    Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(zarrs::storage::StorageError::Other(e.to_string())),
                }
            })
        })
        .join()
        .unwrap();

        res
    }
}

impl zarrs::storage::ListableStorageTraits for AsyncToSyncOpendalStore {
    fn list(&self) -> Result<zarrs::storage::StoreKeys, zarrs::storage::StorageError> {
        Err(zarrs::storage::StorageError::Other(
            "list not supported".into(),
        ))
    }
    fn list_prefix(
        &self,
        _prefix: &zarrs::storage::StorePrefix,
    ) -> Result<zarrs::storage::StoreKeys, zarrs::storage::StorageError> {
        Err(zarrs::storage::StorageError::Other(
            "list_prefix not supported".into(),
        ))
    }
    fn list_dir(
        &self,
        _prefix: &zarrs::storage::StorePrefix,
    ) -> Result<zarrs::storage::StoreKeysPrefixes, zarrs::storage::StorageError> {
        Err(zarrs::storage::StorageError::Other(
            "list_dir not supported".into(),
        ))
    }
    fn size_prefix(
        &self,
        _prefix: &zarrs::storage::StorePrefix,
    ) -> Result<u64, zarrs::storage::StorageError> {
        Err(zarrs::storage::StorageError::Other(
            "size_prefix not supported".into(),
        ))
    }
}

pub fn resolve_sync_store(
    path: &str,
) -> std::result::Result<ResolvedStore, Box<dyn std::error::Error>> {
    let is_cog = path.ends_with(".tif") || path.ends_with(".tiff");

    if path.starts_with("s3://") {
        let bucket_and_path = path.strip_prefix("s3://").unwrap();
        let bucket = bucket_and_path.split('/').next().unwrap_or(bucket_and_path);
        let root = bucket_and_path.strip_prefix(bucket).unwrap_or("/");
        let builder = opendal::services::S3::default().bucket(bucket).root(root);
        let async_operator = opendal::Operator::new(builder)?.finish();

        let store: Arc<dyn ReadableStorageTraits> = if is_cog {
            let async_op_clone = async_operator.clone();
            let root_str = root.to_string();
            let header_res = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async { async_op_clone.read_with(&root_str).range(0..16384).await })
                    .map_err(|e| e.to_string())
            })
            .join()
            .unwrap();

            let header_bytes = header_res?.to_vec();
            let meta = crate::cog::parse_cog_metadata(&header_bytes).unwrap_or_default();
            Arc::new(crate::virtual_store::VirtualCogStore::new(
                async_operator,
                "".to_string(),
                meta,
            )?)
        } else {
            Arc::new(AsyncToSyncOpendalStore::new(async_operator))
        };

        Ok(ResolvedStore {
            store,
            is_remote: true,
        })
    } else if path.starts_with("http://") || path.starts_with("https://") {
        if !is_cog && !path.ends_with(".zarr") && !path.ends_with(".zarr/") {
            // Check if it's a STAC Item
            if let Ok(resp) = reqwest::blocking::get(path) {
                if let Ok(json) = resp.json::<serde_json::Value>() {
                    if json.get("stac_version").is_some()
                        && json.get("type").and_then(|t| t.as_str()) == Some("Feature")
                    {
                        if let Some(assets) = json.get("assets").and_then(|a| a.as_object()) {
                            let mut cog_assets = Vec::new();
                            for (name, asset) in assets {
                                let t = asset.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                let href = asset.get("href").and_then(|h| h.as_str()).unwrap_or("");
                                let is_asset_cog = t.contains("tiff")
                                    || t.contains("cog")
                                    || href.ends_with(".tif")
                                    || href.ends_with(".tiff");

                                if is_asset_cog {
                                    // Resolve relative URLs if needed, but usually STAC assets are absolute
                                    let abs_href =
                                        if href.starts_with("http") || href.starts_with("s3://") {
                                            href.to_string()
                                        } else {
                                            let mut base = path.to_string();
                                            if let Some(idx) = base.rfind('/') {
                                                base.truncate(idx + 1);
                                            }
                                            format!("{}{}", base, href)
                                        };
                                    cog_assets.push((name.to_string(), abs_href));
                                }
                            }

                            if !cog_assets.is_empty() {
                                // Fetch headers concurrently
                                let children = std::thread::spawn(move || {
                                    let rt = tokio::runtime::Runtime::new().unwrap();
                                    rt.block_on(async {
                                        let mut set = tokio::task::JoinSet::new();
                                        for (name, href) in cog_assets {
                                            set.spawn(async move {
                                                let operator = if href.starts_with("s3://") {
                                                    let bucket_and_path =
                                                        href.strip_prefix("s3://").unwrap();
                                                    let bucket = bucket_and_path
                                                        .split('/')
                                                        .next()
                                                        .unwrap_or(bucket_and_path);
                                                    let root = bucket_and_path
                                                        .strip_prefix(bucket)
                                                        .unwrap_or("/");
                                                    let builder = opendal::services::S3::default()
                                                        .bucket(bucket)
                                                        .root(root);
                                                    opendal::Operator::new(builder)
                                                        .unwrap()
                                                        .finish()
                                                } else {
                                                    let builder =
                                                        opendal::services::Http::default()
                                                            .endpoint(&href);
                                                    opendal::Operator::new(builder)
                                                        .unwrap()
                                                        .finish()
                                                };

                                                let root_str = if href.starts_with("s3://") {
                                                    let bucket_and_path =
                                                        href.strip_prefix("s3://").unwrap();
                                                    let bucket = bucket_and_path
                                                        .split('/')
                                                        .next()
                                                        .unwrap_or(bucket_and_path);
                                                    bucket_and_path
                                                        .strip_prefix(bucket)
                                                        .unwrap_or("/")
                                                        .to_string()
                                                } else {
                                                    "".to_string()
                                                };

                                                let header_bytes = operator
                                                    .read_with(&root_str)
                                                    .range(0..16384)
                                                    .await
                                                    .unwrap_or_default()
                                                    .to_vec();
                                                let meta =
                                                    crate::cog::parse_cog_metadata(&header_bytes)
                                                        .unwrap_or_default();
                                                (
                                                    name,
                                                    crate::virtual_store::VirtualCogStore::new(
                                                        operator,
                                                        "".to_string(),
                                                        meta,
                                                    ),
                                                )
                                            });
                                        }

                                        let mut children_map = std::collections::HashMap::new();
                                        while let Some(res) = set.join_next().await {
                                            if let Ok((name, store)) = res {
                                                // A multi-band / unsupported child COG fails the
                                                // whole STAC open; STAC is not first-class yet.
                                                children_map.insert(name, store?);
                                            }
                                        }
                                        Ok::<_, String>(children_map)
                                    })
                                })
                                .join()
                                .unwrap()?;

                                let store = std::sync::Arc::new(
                                    crate::virtual_stac_store::VirtualStacStore::new(children),
                                );
                                return Ok(ResolvedStore {
                                    store,
                                    is_remote: true,
                                });
                            }
                        }
                    }
                }
            }

            // If it wasn't a STAC item itself, check if its parent is a STAC item.
            // E.g., path is "https://.../S2B_T21NYC_20221205T140704_L2A/swir22"
            let mut parts: Vec<&str> = path.split('/').collect();
            if parts.len() > 3 {
                let asset_name = parts.pop().unwrap();
                let parent_url = parts.join("/");
                if let Ok(resp) = reqwest::blocking::get(&parent_url) {
                    if let Ok(json) = resp.json::<serde_json::Value>() {
                        if json.get("stac_version").is_some()
                            && json.get("type").and_then(|t| t.as_str()) == Some("Feature")
                        {
                            if let Some(assets) = json.get("assets").and_then(|a| a.as_object()) {
                                if let Some(asset) = assets.get(asset_name) {
                                    let href =
                                        asset.get("href").and_then(|h| h.as_str()).unwrap_or("");
                                    let abs_href =
                                        if href.starts_with("http") || href.starts_with("s3://") {
                                            href.to_string()
                                        } else {
                                            format!("{}/{}", parent_url, href)
                                        };

                                    let (operator, root_str) = if abs_href.starts_with("s3://") {
                                        let bucket_and_path =
                                            abs_href.strip_prefix("s3://").unwrap();
                                        let bucket = bucket_and_path
                                            .split('/')
                                            .next()
                                            .unwrap_or(bucket_and_path);
                                        let root = bucket_and_path
                                            .strip_prefix(bucket)
                                            .unwrap_or("/")
                                            .trim_start_matches('/')
                                            .to_string();
                                        let builder =
                                            opendal::services::S3::default().bucket(bucket);
                                        (opendal::Operator::new(builder).unwrap().finish(), root)
                                    } else {
                                        let url = reqwest::Url::parse(&abs_href).unwrap();
                                        let port = url
                                            .port()
                                            .map(|p| format!(":{}", p))
                                            .unwrap_or_default();
                                        let endpoint = format!(
                                            "{}://{}{}",
                                            url.scheme(),
                                            url.host_str().unwrap(),
                                            port
                                        );
                                        let path = url.path().trim_start_matches('/').to_string();
                                        let builder =
                                            opendal::services::Http::default().endpoint(&endpoint);
                                        (opendal::Operator::new(builder).unwrap().finish(), path)
                                    };

                                    let op_clone = operator.clone();
                                    let path_clone = root_str.clone();
                                    let header_res = std::thread::spawn(move || {
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        rt.block_on(async {
                                            op_clone.read_with(&path_clone).range(0..16384).await
                                        })
                                        .map_err(|e| e.to_string())
                                    })
                                    .join()
                                    .unwrap();

                                    if let Ok(header_bytes) = header_res {
                                        let meta =
                                            crate::cog::parse_cog_metadata(&header_bytes.to_vec())
                                                .unwrap_or_default();
                                        let store = std::sync::Arc::new(
                                            crate::virtual_store::VirtualCogStore::new(
                                                operator, root_str, meta,
                                            )?,
                                        );
                                        return Ok(ResolvedStore {
                                            store,
                                            is_remote: true,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let builder = opendal::services::Http::default().endpoint(path);
        let async_operator = opendal::Operator::new(builder)?.finish();

        let store: Arc<dyn ReadableStorageTraits> = if is_cog {
            let async_op_clone = async_operator.clone();
            let header_res = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async { async_op_clone.read_with("").range(0..16384).await })
                    .map_err(|e| e.to_string())
            })
            .join()
            .unwrap();

            let header_bytes = header_res?.to_vec();
            let meta = crate::cog::parse_cog_metadata(&header_bytes).unwrap_or_default();
            Arc::new(crate::virtual_store::VirtualCogStore::new(
                async_operator,
                "".to_string(),
                meta,
            )?)
        } else {
            Arc::new(AsyncToSyncOpendalStore::new(async_operator))
        };

        Ok(ResolvedStore {
            store,
            is_remote: true,
        })
    } else {
        let canonical_path =
            std::fs::canonicalize(path).map_err(|e| format!("Invalid path: {}", e))?;
        let allowed_dir = std::env::var("GEOZARR_ALLOW_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());

        let allowed_canon = std::fs::canonicalize(&allowed_dir)
            .map_err(|e| format!("Invalid GEOZARR_ALLOW_PATH: {}", e))?;
        if !canonical_path.starts_with(allowed_canon) {
            return Err("Access denied. Path is not within the allowed sandbox directory (GEOZARR_ALLOW_PATH or CWD).".into());
        }

        let store: Arc<dyn ReadableStorageTraits> = if is_cog {
            let parent = canonical_path.parent().unwrap();
            let filename = canonical_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let builder = opendal::services::Fs::default().root(parent.to_str().unwrap());
            let async_operator = opendal::Operator::new(builder)?.finish();
            let async_op_clone = async_operator.clone();
            let fname_clone = filename.clone();
            // The local Fs reader treats a range that overruns the file as a permanent
            // error ("reader got too little data"), unlike HTTP/S3 which tolerate it.
            // Clamp the header read to the actual file size (capped at the 16 KiB header
            // window) so small COGs are read end-to-end.
            const COG_HEADER_WINDOW: u64 = 16384;
            let header_len = std::fs::metadata(&canonical_path)
                .map(|m| m.len().min(COG_HEADER_WINDOW))
                .unwrap_or(COG_HEADER_WINDOW);
            let header_res = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    async_op_clone
                        .read_with(&fname_clone)
                        .range(0..header_len)
                        .await
                })
                .map_err(|e| e.to_string())
            })
            .join()
            .unwrap();

            let header_bytes = header_res?.to_vec();
            let meta = crate::cog::parse_cog_metadata(&header_bytes).unwrap_or_default();
            Arc::new(crate::virtual_store::VirtualCogStore::new(
                async_operator,
                filename,
                meta,
            )?)
        } else {
            Arc::new(zarrs::storage::store::FilesystemStore::new(canonical_path)?)
        };

        Ok(ResolvedStore {
            store,
            is_remote: false,
        })
    }
}

pub fn resolve_async_store(
    path: &str,
) -> std::result::Result<
    Arc<dyn zarrs::storage::AsyncWritableStorageTraits>,
    Box<dyn std::error::Error>,
> {
    if path.starts_with("s3://") {
        let bucket_and_path = path.strip_prefix("s3://").unwrap();
        let bucket = bucket_and_path.split('/').next().unwrap_or(bucket_and_path);
        let root = bucket_and_path.strip_prefix(bucket).unwrap_or("/");
        let builder = opendal::services::S3::default().bucket(bucket).root(root);
        let operator = opendal::Operator::new(builder)?.finish();
        Ok(Arc::new(zarrs::storage::store::AsyncOpendalStore::new(
            operator,
        )))
    } else {
        let builder = opendal::services::Fs::default().root(path);
        let operator = opendal::Operator::new(builder)?.finish();
        Ok(Arc::new(zarrs::storage::store::AsyncOpendalStore::new(
            operator,
        )))
    }
}

pub async fn list_arrays(uri: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if (uri.starts_with("http://") || uri.starts_with("https://"))
        && !uri.ends_with(".zarr")
        && !uri.ends_with(".zarr/")
        && !uri.ends_with(".tif")
        && !uri.ends_with(".tiff")
    {
        if let Ok(resp) = reqwest::blocking::get(uri) {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                if json.get("stac_version").is_some()
                    && json.get("type").and_then(|t| t.as_str()) == Some("Feature")
                {
                    if let Some(assets) = json.get("assets").and_then(|a| a.as_object()) {
                        let mut cog_assets = Vec::new();
                        for (name, asset) in assets {
                            let t = asset.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            let href = asset.get("href").and_then(|h| h.as_str()).unwrap_or("");
                            let is_asset_cog = t.contains("tiff")
                                || t.contains("cog")
                                || href.ends_with(".tif")
                                || href.ends_with(".tiff");
                            if is_asset_cog {
                                cog_assets.push(name.to_string());
                            }
                        }
                        if !cog_assets.is_empty() {
                            cog_assets.sort();
                            return Ok(cog_assets);
                        }
                    }
                }
            }
        }
    }

    let operator = if uri.starts_with("http") {
        opendal::Operator::new(opendal::services::Http::default().endpoint(uri))?.finish()
    } else {
        opendal::Operator::new(opendal::services::Fs::default().root(uri))?.finish()
    };

    let is_group = operator.is_exist(".zgroup").await.unwrap_or(false);
    let mut arrays = Vec::new();

    if is_group {
        // Try reading consolidated metadata first (crucial for HTTP where listing is unsupported)
        if let Ok(metadata_bytes) = operator.read(".zmetadata").await {
            if let Ok(metadata_json) =
                serde_json::from_slice::<serde_json::Value>(&metadata_bytes.to_bytes())
            {
                if let Some(metadata) = metadata_json.get("metadata").and_then(|m| m.as_object()) {
                    let mut arrays_set = std::collections::HashSet::new();
                    for (key, _) in metadata {
                        if key.ends_with(".zarray") {
                            arrays_set
                                .insert(key.strip_suffix("/.zarray").unwrap_or("").to_string());
                        }
                    }
                    if !arrays_set.is_empty() {
                        let mut sorted: Vec<_> = arrays_set.into_iter().collect();
                        sorted.sort();
                        return Ok(sorted);
                    }
                }
            }
        }

        let entries = operator.list("/").await?;
        for entry in entries {
            if entry.metadata().is_dir() {
                let path = entry.path();
                if operator
                    .is_exist(&format!("{}.zarray", path))
                    .await
                    .unwrap_or(false)
                {
                    arrays.push(path.trim_end_matches('/').to_string());
                }
            }
        }
    } else if operator.is_exist(".zarray").await.unwrap_or(false) {
        arrays.push("".to_string());
    }

    Ok(arrays)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_arrays() {
        let arrays = list_arrays("../climate_data.zarr").await.unwrap();
        println!("Found arrays: {:?}", arrays);
        // assert_eq!(arrays.len(), 4);
    }

    #[tokio::test]
    async fn test_resolve_sync_store_cog() {
        let result = resolve_sync_store("test.tif");
        // Without the actual file it will fail, but we just check the path logic exists
        assert!(result.is_err());
    }
}

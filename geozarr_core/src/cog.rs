pub struct TiffHeader {
    pub is_little_endian: bool,
    pub first_ifd_offset: u32,
}

pub fn parse_tiff_header(buffer: &[u8]) -> Result<TiffHeader, String> {
    if buffer.len() < 8 {
        return Err("Buffer too small for TIFF header".into());
    }

    let is_little_endian = match &buffer[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err("Invalid TIFF byte order".into()),
    };

    let magic = if is_little_endian {
        u16::from_le_bytes(buffer[2..4].try_into().unwrap())
    } else {
        u16::from_be_bytes(buffer[2..4].try_into().unwrap())
    };

    if magic != 42 && magic != 43 {
        // BigTIFF is 43, classic is 42
        return Err("Invalid TIFF magic number".into());
    }

    let first_ifd_offset = if is_little_endian {
        u32::from_le_bytes(buffer[4..8].try_into().unwrap())
    } else {
        u32::from_be_bytes(buffer[4..8].try_into().unwrap())
    };

    Ok(TiffHeader {
        is_little_endian,
        first_ifd_offset,
    })
}

#[derive(Debug, PartialEq)]
pub enum CogCompression {
    None,
    Deflate,
}

#[derive(Debug, Default, Clone)]
pub struct CogMetadata {
    pub image_width: u32,
    pub image_length: u32,
    pub tile_width: u32,
    pub tile_length: u32,
    pub tile_offsets: Vec<u64>,
    pub tile_byte_counts: Vec<u64>,
    pub is_little_endian: bool,
    pub bits_per_sample: u16,           // 258; default filled in Step 3
    pub sample_format: u16,             // 339; 1=uint (default), 2=int, 3=float
    pub samples_per_pixel: u16,         // 277; default 1
    pub compression: u16,               // 259; 1=none (default)
    pub predictor: u16,                 // 317; 1=none (default)
    pub nodata: Option<f64>,            // GDAL_NODATA tag 42113
    pub pixel_scale: Vec<f64>,          // ModelPixelScale 33550
    pub tiepoint: Vec<f64>,             // ModelTiepoint 33922
    pub model_transformation: Vec<f64>, // ModelTransformation 34264
    pub epsg: Option<u32>,              // from GeoKeyDirectory 34735
}

impl CogMetadata {
    /// Numpy/Zarr-V2 dtype string for this COG's single band, e.g. "<i2".
    /// Errors on multi-band or unsupported bit-depth/sample-format combinations.
    pub fn zarr_dtype(&self) -> Result<String, String> {
        if self.samples_per_pixel != 1 {
            return Err(format!(
                "multi-band COGs not yet supported (SamplesPerPixel={})",
                self.samples_per_pixel
            ));
        }
        let endian = if self.bits_per_sample <= 8 {
            "|"
        } else if self.is_little_endian {
            "<"
        } else {
            ">"
        };
        let kind = match self.sample_format {
            3 => "f", // float
            2 => "i", // signed int
            1 => "u", // unsigned int
            other => return Err(format!("unsupported TIFF SampleFormat {other}")),
        };
        let bytes = match self.bits_per_sample {
            8 => 1,
            16 => 2,
            32 => 4,
            64 => 8,
            other => return Err(format!("unsupported BitsPerSample {other}")),
        };
        if kind == "f" && bytes < 4 {
            return Err(format!(
                "unsupported float width {} bits",
                self.bits_per_sample
            ));
        }
        Ok(format!("{endian}{kind}{bytes}"))
    }

    /// Resolve the TIFF Compression+Predictor tags to a supported kind, or error.
    pub fn compression_kind(&self) -> Result<CogCompression, String> {
        let comp = match self.compression {
            1 => CogCompression::None,
            8 | 32946 => CogCompression::Deflate,
            other => {
                return Err(format!(
                "unsupported COG compression {other} (only uncompressed and Deflate are supported)"
            ))
            }
        };
        if self.predictor != 1 {
            return Err(format!(
                "unsupported COG predictor {} (only predictor=1/none is supported)",
                self.predictor
            ));
        }
        Ok(comp)
    }

    /// Parse a GDAL_NODATA ASCII tag value to a number (returns None for NaN/unparseable).
    pub fn parse_nodata(s: &str) -> Option<f64> {
        let t = s.trim().trim_end_matches('\0').trim();
        match t.parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => None,
        }
    }

    /// North-up affine as a SpatialTransform with dims [row(lat), col(lon)].
    pub fn spatial_transform(&self) -> Option<crate::metadata::SpatialTransform> {
        if self.pixel_scale.len() >= 2 && self.tiepoint.len() >= 6 {
            let sx = self.pixel_scale[0];
            let sy = self.pixel_scale[1];
            let tx = self.tiepoint[3];
            let ty = self.tiepoint[4];
            return Some(crate::metadata::SpatialTransform {
                scale: vec![-sy, sx],
                translation: vec![ty, tx],
            });
        }
        if self.model_transformation.len() >= 16 {
            // 4x4 row-major: x' = a*col + b*row + ... ; use diagonal + translation column
            let m = &self.model_transformation;
            let sx = m[0]; // col scale (x)
            let sy = m[5]; // row scale (y), already signed
            let tx = m[3];
            let ty = m[7];
            return Some(crate::metadata::SpatialTransform {
                scale: vec![sy, sx],
                translation: vec![ty, tx],
            });
        }
        None
    }

    pub fn crs(&self) -> Option<String> {
        self.epsg.map(|c| format!("EPSG:{c}"))
    }

    /// Geographic CRS → ["lat","lon"] (enables lat_min/lon_max pushdown);
    /// any other/absent CRS → ["y","x"].
    pub fn dim_names(&self) -> Vec<String> {
        match self.epsg {
            Some(4326) => vec!["lat".into(), "lon".into()],
            _ => vec!["y".into(), "x".into()],
        }
    }
}

pub fn parse_cog_metadata(buffer: &[u8]) -> Result<CogMetadata, String> {
    let header = parse_tiff_header(buffer)?;
    let mut meta = CogMetadata {
        is_little_endian: header.is_little_endian,
        ..Default::default()
    };

    let mut offset = header.first_ifd_offset as usize;
    if offset + 2 > buffer.len() {
        return Err("IFD offset out of bounds".into());
    }

    let num_entries = if header.is_little_endian {
        u16::from_le_bytes(buffer[offset..offset + 2].try_into().unwrap())
    } else {
        u16::from_be_bytes(buffer[offset..offset + 2].try_into().unwrap())
    };
    offset += 2;

    for _ in 0..num_entries {
        if offset + 12 > buffer.len() {
            break;
        }

        let tag = if header.is_little_endian {
            u16::from_le_bytes(buffer[offset..offset + 2].try_into().unwrap())
        } else {
            u16::from_be_bytes(buffer[offset..offset + 2].try_into().unwrap())
        };

        let typ = if header.is_little_endian {
            u16::from_le_bytes(buffer[offset + 2..offset + 4].try_into().unwrap())
        } else {
            u16::from_be_bytes(buffer[offset + 2..offset + 4].try_into().unwrap())
        };

        let count = if header.is_little_endian {
            u32::from_le_bytes(buffer[offset + 4..offset + 8].try_into().unwrap())
        } else {
            u32::from_be_bytes(buffer[offset + 4..offset + 8].try_into().unwrap())
        };

        let val_or_offset = if header.is_little_endian {
            u32::from_le_bytes(buffer[offset + 8..offset + 12].try_into().unwrap())
        } else {
            u32::from_be_bytes(buffer[offset + 8..offset + 12].try_into().unwrap())
        };

        let extract_single_val = || -> u32 {
            if typ == 3 {
                // SHORT
                if header.is_little_endian {
                    u16::from_le_bytes(buffer[offset + 8..offset + 10].try_into().unwrap()) as u32
                } else {
                    u16::from_be_bytes(buffer[offset + 8..offset + 10].try_into().unwrap()) as u32
                }
            } else {
                val_or_offset
            }
        };

        let extract_array = |count: usize, offset_val: u32| -> Vec<u64> {
            let mut res = Vec::with_capacity(count);
            let mut ptr = offset_val as usize;
            for _ in 0..count {
                if ptr + 4 > buffer.len() {
                    break;
                }
                let v = if typ == 3 {
                    let sv = if header.is_little_endian {
                        u16::from_le_bytes(buffer[ptr..ptr + 2].try_into().unwrap()) as u64
                    } else {
                        u16::from_be_bytes(buffer[ptr..ptr + 2].try_into().unwrap()) as u64
                    };
                    ptr += 2;
                    sv
                } else {
                    let lv = if header.is_little_endian {
                        u32::from_le_bytes(buffer[ptr..ptr + 4].try_into().unwrap()) as u64
                    } else {
                        u32::from_be_bytes(buffer[ptr..ptr + 4].try_into().unwrap()) as u64
                    };
                    ptr += 4;
                    lv
                };
                res.push(v);
            }
            res
        };

        let extract_f64_array = |count: usize, offset_val: u32| -> Vec<f64> {
            let mut res = Vec::with_capacity(count);
            let mut ptr = offset_val as usize;
            for _ in 0..count {
                if ptr + 8 > buffer.len() {
                    break;
                }
                let v = if header.is_little_endian {
                    f64::from_le_bytes(buffer[ptr..ptr + 8].try_into().unwrap())
                } else {
                    f64::from_be_bytes(buffer[ptr..ptr + 8].try_into().unwrap())
                };
                ptr += 8;
                res.push(v);
            }
            res
        };

        let extract_u16_array = |count: usize, offset_val: u32| -> Vec<u16> {
            let mut res = Vec::with_capacity(count);
            let mut ptr = offset_val as usize;
            for _ in 0..count {
                if ptr + 2 > buffer.len() {
                    break;
                }
                let v = if header.is_little_endian {
                    u16::from_le_bytes(buffer[ptr..ptr + 2].try_into().unwrap())
                } else {
                    u16::from_be_bytes(buffer[ptr..ptr + 2].try_into().unwrap())
                };
                ptr += 2;
                res.push(v);
            }
            res
        };

        match tag {
            256 => meta.image_width = extract_single_val(),
            257 => meta.image_length = extract_single_val(),
            322 => meta.tile_width = extract_single_val(),
            323 => meta.tile_length = extract_single_val(),
            258 => meta.bits_per_sample = extract_single_val() as u16,
            277 => meta.samples_per_pixel = extract_single_val() as u16,
            339 => meta.sample_format = extract_single_val() as u16,
            259 => meta.compression = extract_single_val() as u16,
            317 => meta.predictor = extract_single_val() as u16,
            42113 => {
                let n = count as usize;
                let raw: &[u8] = if n <= 4 {
                    // ASCII value <= 4 bytes is stored inline in the value/offset field.
                    &buffer[offset + 8..offset + 8 + n]
                } else {
                    let start = val_or_offset as usize;
                    let end = (start + n).min(buffer.len());
                    if start > end {
                        &[]
                    } else {
                        &buffer[start..end]
                    }
                };
                if let Ok(s) = std::str::from_utf8(raw) {
                    meta.nodata = CogMetadata::parse_nodata(s);
                }
            }
            33550 => meta.pixel_scale = extract_f64_array(count as usize, val_or_offset),
            33922 => meta.tiepoint = extract_f64_array(count as usize, val_or_offset),
            34264 => meta.model_transformation = extract_f64_array(count as usize, val_or_offset),
            34735 => {
                let keys = extract_u16_array(count as usize, val_or_offset);
                // header is keys[0..4]; entries are 4-tuples [KeyID, Location, Count, Value]
                let mut i = 4;
                while i + 4 <= keys.len() {
                    let key_id = keys[i];
                    let location = keys[i + 1];
                    let value = keys[i + 3];
                    if (key_id == 3072 || key_id == 2048) && location == 0 {
                        meta.epsg = Some(value as u32);
                    }
                    i += 4;
                }
            }
            324 => {
                if count == 1 {
                    meta.tile_offsets.push(extract_single_val() as u64);
                } else {
                    meta.tile_offsets = extract_array(count as usize, val_or_offset);
                }
            }
            325 => {
                if count == 1 {
                    meta.tile_byte_counts.push(extract_single_val() as u64);
                } else {
                    meta.tile_byte_counts = extract_array(count as usize, val_or_offset);
                }
            }
            _ => {}
        }
        offset += 12;
    }

    // Apply TIFF defaults for absent tags.
    if meta.bits_per_sample == 0 {
        meta.bits_per_sample = 32;
    }
    if meta.sample_format == 0 {
        meta.sample_format = 1; // unsigned int
    }
    if meta.samples_per_pixel == 0 {
        meta.samples_per_pixel = 1;
    }
    if meta.compression == 0 {
        meta.compression = 1; // none
    }
    if meta.predictor == 0 {
        meta.predictor = 1; // none
    }

    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tiff_header() {
        // Little-endian TIFF header (II), 42, IFD offset = 8
        let buffer: &[u8] = &[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00];
        let header = parse_tiff_header(buffer).unwrap();
        assert!(header.is_little_endian);
        assert_eq!(header.first_ifd_offset, 8);
    }

    #[test]
    fn test_parse_ifd() {
        // Dummy buffer with a simple IFD at offset 8 containing 1 tag (ImageWidth)
        // Tag: 256 (ImageWidth), Type: 4 (LONG), Count: 1, Value: 1024
        let mut buffer = vec![0; 32];
        buffer[0..2].copy_from_slice(b"II"); // LE
        buffer[2..4].copy_from_slice(&42u16.to_le_bytes()); // Magic
        buffer[4..8].copy_from_slice(&8u32.to_le_bytes()); // Offset=8

        // IFD starts at 8
        buffer[8..10].copy_from_slice(&1u16.to_le_bytes()); // 1 entry
                                                            // Entry 0 starts at 10
        buffer[10..12].copy_from_slice(&256u16.to_le_bytes()); // Tag=ImageWidth
        buffer[12..14].copy_from_slice(&4u16.to_le_bytes()); // Type=LONG
        buffer[14..18].copy_from_slice(&1u32.to_le_bytes()); // Count=1
        buffer[18..22].copy_from_slice(&1024u32.to_le_bytes()); // Value=1024

        let info = parse_cog_metadata(&buffer).unwrap();
        assert_eq!(info.image_width, 1024);
    }

    #[test]
    fn test_parse_scalar_tags() {
        // II, magic 42, IFD at 8; 6 entries: width,length,tilew,tilel,bits,sampfmt
        let mut b = vec![0u8; 100];
        b[0..2].copy_from_slice(b"II");
        b[2..4].copy_from_slice(&42u16.to_le_bytes());
        b[4..8].copy_from_slice(&8u32.to_le_bytes());
        b[8..10].copy_from_slice(&6u16.to_le_bytes()); // 6 entries
        let mut o = 10;
        let put = |b: &mut [u8], o: usize, tag: u16, typ: u16, val: u32| {
            b[o..o + 2].copy_from_slice(&tag.to_le_bytes());
            b[o + 2..o + 4].copy_from_slice(&typ.to_le_bytes());
            b[o + 4..o + 8].copy_from_slice(&1u32.to_le_bytes());
            b[o + 8..o + 12].copy_from_slice(&val.to_le_bytes());
        };
        put(&mut b, o, 256, 4, 4);
        o += 12; // ImageWidth=4
        put(&mut b, o, 257, 4, 2);
        o += 12; // ImageLength=2
        put(&mut b, o, 322, 3, 4);
        o += 12; // TileWidth=4 (SHORT)
        put(&mut b, o, 323, 3, 2);
        o += 12; // TileLength=2
        put(&mut b, o, 258, 3, 16);
        o += 12; // BitsPerSample=16
        put(&mut b, o, 339, 3, 2); // SampleFormat=2 (signed int)
        let m = parse_cog_metadata(&b).unwrap();
        assert!(m.is_little_endian);
        assert_eq!(m.bits_per_sample, 16);
        assert_eq!(m.sample_format, 2);
        assert_eq!(m.samples_per_pixel, 1); // defaulted
        assert_eq!(m.compression, 1); // defaulted
    }

    #[test]
    fn test_zarr_dtype_and_band_guard() {
        let mut m = CogMetadata {
            is_little_endian: true,
            samples_per_pixel: 1,
            ..Default::default()
        };
        m.bits_per_sample = 16;
        m.sample_format = 2;
        assert_eq!(m.zarr_dtype().unwrap(), "<i2");
        m.bits_per_sample = 32;
        m.sample_format = 3;
        assert_eq!(m.zarr_dtype().unwrap(), "<f4");
        m.bits_per_sample = 8;
        m.sample_format = 1;
        assert_eq!(m.zarr_dtype().unwrap(), "|u1");
        // big-endian flips the prefix
        m.is_little_endian = false;
        m.bits_per_sample = 16;
        m.sample_format = 1;
        assert_eq!(m.zarr_dtype().unwrap(), ">u2");
        // multi-band is rejected
        m.samples_per_pixel = 3;
        assert!(m.zarr_dtype().is_err());
        // unsupported bit depth rejected
        let bad = CogMetadata {
            samples_per_pixel: 1,
            bits_per_sample: 12,
            sample_format: 1,
            ..Default::default()
        };
        assert!(bad.zarr_dtype().is_err());
    }

    #[test]
    fn test_compression_and_predictor_and_nodata() {
        let mut m = CogMetadata {
            compression: 1,
            predictor: 1,
            ..Default::default()
        };
        assert!(matches!(m.compression_kind(), Ok(CogCompression::None)));
        m.compression = 8;
        assert!(matches!(m.compression_kind(), Ok(CogCompression::Deflate)));
        m.compression = 32946; // old-style deflate
        assert!(matches!(m.compression_kind(), Ok(CogCompression::Deflate)));
        m.compression = 5; // LZW
        assert!(m.compression_kind().is_err());
        // predictor != 1 with deflate is rejected
        m.compression = 8;
        m.predictor = 2;
        assert!(m.compression_kind().is_err());
        // nodata parse
        assert_eq!(CogMetadata::parse_nodata("  -9999  "), Some(-9999.0));
        assert_eq!(CogMetadata::parse_nodata("nan"), None);
    }

    #[test]
    fn test_georeferencing() {
        use crate::metadata::SpatialTransform;
        // ModelPixelScale=[2.0,2.0,0], ModelTiepoint=[0,0,0, -180,90,0], EPSG via GeoKey 2048=4326
        let mut b = vec![0u8; 300];
        b[0..2].copy_from_slice(b"II");
        b[2..4].copy_from_slice(&42u16.to_le_bytes());
        b[4..8].copy_from_slice(&8u32.to_le_bytes());
        b[8..10].copy_from_slice(&3u16.to_le_bytes()); // 3 entries
        let mut o = 10;
        // entry helper for arrays pointing at an offset
        let put = |b: &mut [u8], o: usize, tag: u16, typ: u16, count: u32, voff: u32| {
            b[o..o + 2].copy_from_slice(&tag.to_le_bytes());
            b[o + 2..o + 4].copy_from_slice(&typ.to_le_bytes());
            b[o + 4..o + 8].copy_from_slice(&count.to_le_bytes());
            b[o + 8..o + 12].copy_from_slice(&voff.to_le_bytes());
        };
        // place data after the IFD (IFD ends at 10 + 3*12 = 46)
        let scale_off = 48usize; // 3 doubles = 24 bytes
        for (i, v) in [2.0f64, 2.0, 0.0].iter().enumerate() {
            b[scale_off + i * 8..scale_off + i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        let tp_off = 72usize; // 6 doubles = 48 bytes
        for (i, v) in [0.0f64, 0.0, 0.0, -180.0, 90.0, 0.0].iter().enumerate() {
            b[tp_off + i * 8..tp_off + i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        let gk_off = 120usize; // GeoKeyDirectory SHORTs: header(4) + 1 key(4) = 8 shorts
        let gk: [u16; 8] = [1, 1, 0, 1, 2048, 0, 1, 4326];
        for (i, v) in gk.iter().enumerate() {
            b[gk_off + i * 2..gk_off + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        put(&mut b, o, 33550, 12, 3, scale_off as u32);
        o += 12; // ModelPixelScale (DOUBLE)
        put(&mut b, o, 33922, 12, 6, tp_off as u32);
        o += 12; // ModelTiepoint (DOUBLE)
        put(&mut b, o, 34735, 3, 8, gk_off as u32); // GeoKeyDirectory (SHORT)
        let m = parse_cog_metadata(&b).unwrap();
        let t: SpatialTransform = m.spatial_transform().unwrap();
        assert_eq!(t.scale, vec![-2.0, 2.0]); // [lat(row), lon(col)]
        assert_eq!(t.translation, vec![90.0, -180.0]); // [tiepointY, tiepointX]
        assert_eq!(m.crs(), Some("EPSG:4326".to_string()));
        assert_eq!(m.dim_names(), vec!["lat".to_string(), "lon".to_string()]);
    }

    #[test]
    fn test_nodata_inline() {
        // GDAL_NODATA (42113), ASCII (type 2), count=2 value "0\0" stored INLINE
        // (<= 4 bytes lives in the entry's value/offset field).
        let mut b = vec![0u8; 32];
        b[0..2].copy_from_slice(b"II");
        b[2..4].copy_from_slice(&42u16.to_le_bytes());
        b[4..8].copy_from_slice(&8u32.to_le_bytes());
        b[8..10].copy_from_slice(&1u16.to_le_bytes()); // 1 entry
                                                       // Entry at offset 10
        b[10..12].copy_from_slice(&42113u16.to_le_bytes()); // GDAL_NODATA
        b[12..14].copy_from_slice(&2u16.to_le_bytes()); // Type=ASCII
        b[14..18].copy_from_slice(&2u32.to_le_bytes()); // Count=2 ("0\0")
                                                        // Inline value "0\0" in first two bytes of value field (offset 18..20)
        b[18] = 0x30; // '0'
        b[19] = 0x00; // NUL
        let m = parse_cog_metadata(&b).unwrap();
        assert_eq!(m.nodata, Some(0.0));
    }

    #[test]
    fn test_nodata_offset() {
        // GDAL_NODATA (42113), ASCII (type 2), count=6 value "-9999\0" stored at an OFFSET
        // (> 4 bytes lives at a file offset pointed to by the value field).
        let mut b = vec![0u8; 64];
        b[0..2].copy_from_slice(b"II");
        b[2..4].copy_from_slice(&42u16.to_le_bytes());
        b[4..8].copy_from_slice(&8u32.to_le_bytes());
        b[8..10].copy_from_slice(&1u16.to_le_bytes()); // 1 entry
                                                       // Entry at offset 10; IFD ends at 22
        let data_off = 32usize;
        b[10..12].copy_from_slice(&42113u16.to_le_bytes()); // GDAL_NODATA
        b[12..14].copy_from_slice(&2u16.to_le_bytes()); // Type=ASCII
        b[14..18].copy_from_slice(&6u32.to_le_bytes()); // Count=6 ("-9999\0")
        b[18..22].copy_from_slice(&(data_off as u32).to_le_bytes()); // offset to value
        b[data_off..data_off + 6].copy_from_slice(b"-9999\0");
        let m = parse_cog_metadata(&b).unwrap();
        assert_eq!(m.nodata, Some(-9999.0));
    }
}
